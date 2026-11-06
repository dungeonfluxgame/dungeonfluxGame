use super::fixtures::*;
use super::{ResponseError, current_responses, validate_current_submission};
use df_model::checkpoint::*;
use df_model::commands::{CommandError, CommandLimits};
use df_types::{OperationId, RunId, SessionId};

pub(super) fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 8,
        maximum_text_bytes: 32,
        maximum_retained_bytes: 8192,
    }
}

pub(super) fn request(reaction: bool) -> CommandInput {
    let pending = pending();
    let command = if reaction {
        GameCommand::SelectReaction {
            resolution: pending.id,
            window: pending.window.id,
            offer: label("fixture-offer-1"),
            option: label("fixture-option-1"),
        }
    } else {
        GameCommand::SelectChoice {
            resolution: pending.id,
            window: pending.window.id,
            offer: label("fixture-offer-1"),
            option: label("fixture-option-1"),
        }
    };
    CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        member: member(3),
        command,
    }
}

pub(super) fn waiting(reaction: bool, change: impl FnOnce(&mut GameState)) -> Checkpoint {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
    let mut pending = pending();
    if !reaction && let PendingInput::Reaction { remaining } = pending.next {
        pending.next = PendingInput::Choice { remaining };
    }
    supplied.pending.push(pending);
    change(&mut supplied);
    checkpoint(supplied).unwrap()
}

fn offers<'a>(
    request: &CommandInput,
    current: &'a Checkpoint,
) -> Result<Vec<&'a OfferedResponse>, ResponseError> {
    current_responses(
        request,
        current,
        current.basis(),
        current.pins(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        command_limits(),
    )
}

fn submit(request: CommandInput, current: &Checkpoint) -> Result<(), ResponseError> {
    validate_current_submission(
        &GameInput::Game(request),
        current,
        current.basis(),
        current.pins(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        command_limits(),
    )
}

#[test]
fn every_current_choice_and_reaction_option_passes_the_same_submission_admission() {
    for reaction in [false, true] {
        let current = waiting(reaction, |state| {
            let pending = state.pending.first_mut().unwrap();
            let remaining = match &mut pending.next {
                PendingInput::Choice { remaining } | PendingInput::Reaction { remaining } => {
                    remaining
                }
                _ => unreachable!(),
            };
            remaining
                .first_mut()
                .unwrap()
                .options
                .push(label("fixture-option-2"));
        });
        let before = current.clone();
        let template = request(reaction);
        let responses = offers(&template, &current).unwrap();
        assert_eq!(responses.len(), 1);
        let response = responses.first().unwrap();
        assert_eq!(response.source, rule());
        for option in &response.options {
            let mut selected = template.clone();
            match &mut selected.command {
                GameCommand::SelectChoice {
                    offer,
                    option: selected_option,
                    ..
                }
                | GameCommand::SelectReaction {
                    offer,
                    option: selected_option,
                    ..
                } => {
                    *offer = response.offer.clone();
                    *selected_option = option.clone();
                }
                _ => unreachable!(),
            }
            assert_eq!(submit(selected, &current), Ok(()));
        }
        assert_eq!(current, before);
    }
}

#[test]
fn enumeration_uses_current_records_instead_of_template_offer_and_option_labels() {
    let current = waiting(false, |_| {});
    let mut template = request(false);
    if let GameCommand::SelectChoice { offer, option, .. } = &mut template.command {
        *offer = label("unoffered");
        *option = label("unoffered");
    }
    assert_eq!(
        offers(&template, &current).unwrap().first().unwrap().offer,
        label("fixture-offer-1")
    );
    assert_eq!(
        submit(template, &current),
        Err(ResponseError::Admission(CommandError::UnofferedResponse))
    );
}

#[test]
fn older_client_sequence_is_admitted_when_exact_current_response_survives() {
    let current = waiting(true, |_| {});
    let mut selected = request(true);
    selected.basis.revision = revision(2, 1);
    selected.observed_revision = revision(2, 1);
    assert_eq!(offers(&selected, &current).unwrap().len(), 1);
    assert_eq!(submit(selected, &current), Ok(()));
}

#[test]
fn future_and_retired_client_revisions_fail_both_paths_without_changes() {
    let current = waiting(false, |_| {});
    let before = current.clone();
    for stale in [revision(2, 9), revision(1, 8)] {
        for observed in [false, true] {
            let mut selected = request(false);
            if observed {
                selected.observed_revision = stale;
            } else {
                selected.basis.revision = stale;
            }
            assert_eq!(
                offers(&selected, &current),
                Err(ResponseError::Admission(CommandError::StaleRevision))
            );
            assert_eq!(
                submit(selected, &current),
                Err(ResponseError::Admission(CommandError::StaleRevision))
            );
        }
    }
    assert_eq!(current, before);
}

#[test]
fn changed_pending_owner_and_retired_window_refuse_previous_submission() {
    for changed_owner in [false, true] {
        let current = waiting(false, |state| {
            if changed_owner {
                state.members.push(MembershipLink {
                    member: member(12),
                    character: None,
                });
                if let PendingInput::Choice { remaining } =
                    &mut state.pending.first_mut().unwrap().next
                {
                    remaining.first_mut().unwrap().participant = member(12);
                }
            } else {
                state.pending.clear();
            }
        });
        let before = current.clone();
        let expected = if changed_owner {
            CommandError::UnofferedResponse
        } else {
            CommandError::StaleWindow
        };
        assert_eq!(
            offers(&request(false), &current),
            Err(ResponseError::Admission(expected))
        );
        assert_eq!(
            submit(request(false), &current),
            Err(ResponseError::Admission(expected))
        );
        assert_eq!(current, before);
    }
}

#[test]
fn changed_option_is_refused_and_other_participants_offers_are_filtered() {
    let current = waiting(false, |state| {
        state.members.push(MembershipLink {
            member: member(12),
            character: None,
        });
        if let PendingInput::Choice { remaining } = &mut state.pending.first_mut().unwrap().next {
            remaining.first_mut().unwrap().options = vec![label("replacement-option")];
            remaining.push(OfferedResponse {
                participant: member(12),
                offer: label("private-other-offer"),
                options: vec![label("private-other-option")],
                source: rule(),
            });
        }
    });
    let template = request(false);
    let available = offers(&template, &current).unwrap();
    assert_eq!(available.len(), 1);
    assert_eq!(available.first().unwrap().participant, template.member);
    assert_eq!(
        available.first().unwrap().options,
        vec![label("replacement-option")]
    );
    assert_eq!(
        submit(template, &current),
        Err(ResponseError::Admission(CommandError::UnofferedResponse))
    );
}

#[test]
fn stale_trusted_snapshot_and_changed_handler_pins_refuse_both_paths() {
    let current = waiting(false, |_| {});
    let mut stale_basis = current.basis();
    stale_basis.revision = revision(2, 7);
    let mut changed_pins = pins();
    changed_pins.rules.handler = label("changed-handler");
    for (trusted_basis, trusted_pins, expected) in [
        (stale_basis, pins(), CheckpointError::StaleBasis),
        (
            current.basis(),
            changed_pins,
            CheckpointError::RulesMismatch,
        ),
    ] {
        assert_eq!(
            current_responses(
                &request(false),
                &current,
                trusted_basis,
                &trusted_pins,
                ReferenceInventory {
                    rules: &[rule()],
                    content: &[content()],
                    resources: &[],
                    assets: &[]
                },
                command_limits()
            ),
            Err(ResponseError::Snapshot(expected))
        );
        assert_eq!(
            validate_current_submission(
                &GameInput::Game(request(false)),
                &current,
                trusted_basis,
                &trusted_pins,
                ReferenceInventory {
                    rules: &[rule()],
                    content: &[content()],
                    resources: &[],
                    assets: &[]
                },
                command_limits()
            ),
            Err(ResponseError::Snapshot(expected))
        );
    }
}

#[test]
fn wrong_pending_kind_and_nonresponse_command_do_not_become_admitted_options() {
    let current = waiting(true, |_| {});
    assert_eq!(
        offers(&request(false), &current),
        Err(ResponseError::Admission(CommandError::WrongPendingKind))
    );
    assert_eq!(
        submit(request(false), &current),
        Err(ResponseError::Admission(CommandError::WrongPendingKind))
    );
    let mut unsupported = request(true);
    unsupported.command = GameCommand::SubmitRoll {
        resolution: pending().id,
        window: pending().window.id,
    };
    assert_eq!(
        offers(&unsupported, &current),
        Err(ResponseError::UnsupportedCommand)
    );
    assert_eq!(
        submit(unsupported, &current),
        Err(ResponseError::UnsupportedCommand)
    );
}

#[test]
fn enumeration_refuses_option_count_and_admission_byte_bounds_without_partial_output() {
    let current = waiting(false, |state| {
        if let PendingInput::Choice { remaining } = &mut state.pending.first_mut().unwrap().next {
            remaining
                .first_mut()
                .unwrap()
                .options
                .push(label("fixture-option-2"));
        }
    });
    for bounds in [
        CommandLimits {
            maximum_records: 1,
            ..command_limits()
        },
        CommandLimits {
            maximum_retained_bytes: 1,
            ..command_limits()
        },
    ] {
        assert_eq!(
            current_responses(
                &request(false),
                &current,
                current.basis(),
                current.pins(),
                ReferenceInventory {
                    rules: &[rule()],
                    content: &[content()],
                    resources: &[],
                    assets: &[]
                },
                bounds
            ),
            Err(ResponseError::Admission(CommandError::Capacity))
        );
    }
}

#[test]
fn wrong_client_session_and_run_fail_both_paths() {
    let current = waiting(false, |_| {});
    for wrong_session in [false, true] {
        let mut selected = request(false);
        let expected = if wrong_session {
            selected.basis.session = SessionId::from_bytes(&[21; 16]).unwrap();
            CommandError::WrongSession
        } else {
            selected.basis.run = RunId::from_bytes(&[22; 16]).unwrap();
            CommandError::WrongRun
        };
        assert_eq!(
            offers(&selected, &current),
            Err(ResponseError::Admission(expected))
        );
        assert_eq!(
            submit(selected, &current),
            Err(ResponseError::Admission(expected))
        );
    }
}
