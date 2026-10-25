use super::*;
use df_rules::{DispatchError, InvocationError};
use df_types::{BuildRevision, OperationId};

fn request(current: &Checkpoint, die_value: u32) -> (GameInput, ActualDraw) {
    let operation = OperationId::from_bytes(&[0x81; 16]).unwrap();
    let source = rule().unwrap();
    (
        GameInput::Game(CommandInput {
            basis: current.basis(),
            observed_revision: current.basis().revision,
            operation,
            member: member().unwrap(),
            command: GameCommand::ProposeAction {
                actor: actor().unwrap(),
                action: content("inspect-seal").unwrap(),
                targets: vec![],
                choices: vec![],
            },
        }),
        ActualDraw {
            operation,
            ordinal: 0,
            resolution: ResolutionId::from_bytes(&[0x61; 16]).unwrap(),
            window: WindowId::from_bytes(&[0x62; 16]).unwrap(),
            sides: 20,
            value: die_value,
            source,
        },
    )
}

#[test]
fn native_registered_ability_check_explanation_identifies_actual_selected_semantics() {
    let current = initial().unwrap();
    let before = current.clone();
    let pins = pins().unwrap();
    let source = rule().unwrap();
    let selector = label("harbor-normal-ability-check-1").unwrap();
    let handler = HarborHandler { pins: pins.clone() };
    let manifest = source_manifest();
    let entries = [CatalogEntry::new(&source, &manifest)];
    let catalog = CatalogSnapshot::from_published(
        &pins.rules.catalog,
        &pins,
        &manifest,
        &entries,
        CatalogLimits {
            max_complete_bytes: 4096,
            max_entries: 1,
            max_item_bytes: 4096,
            max_total_item_bytes: 4096,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(&selector, &source, &handler)];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();

    for (die, expected_entry) in [
        (10, "seal-obscured"),
        (11, "clue-loading-pier"),
        (1, "seal-obscured"),
        (20, "clue-loading-pier"),
    ] {
        let (command, draw) = request(&current, die);
        let (candidate, explanation) = registry
            .stage_with_provenance(
                &pins,
                &selector,
                &source,
                RulesCommandInput {
                    command: &command,
                    supplied_draws: std::slice::from_ref(&draw),
                },
                &current,
                1024 * 1024,
            )
            .unwrap();
        assert_eq!(explanation.source(), &source);
        assert_eq!(explanation.selector(), &selector);
        assert_eq!(explanation.rules_pins().mode, RulesMode::Standard2024);
        assert_eq!(explanation.source().source.as_str(), "srd521-page6");
        assert_eq!(
            explanation.source().clause.as_str(),
            "normal-proficient-total-versus-dc"
        );
        assert_eq!(explanation.source_bytes(), manifest);
        assert_ne!(explanation.source_bytes(), CONTENT);
        assert_eq!(
            ContentDigest(Sha256::digest(explanation.source_bytes()).into()),
            explanation.rules_pins().source_manifest_digest
        );
        assert_eq!(explanation.rules_pins(), &candidate.pins().rules);
        assert_eq!(explanation.content_pins(), &candidate.pins().content);
        assert_eq!(explanation.build(), &candidate.pins().build);
        // These are supplied labels in the native example, not a claim of executed WASM checks.
        for revision in [
            BuildRevision::Source,
            BuildRevision::Native,
            BuildRevision::Wasm,
            BuildRevision::Configuration,
            BuildRevision::Content,
        ] {
            assert_eq!(
                explanation.build().revision(revision),
                current.pins().build.revision(revision)
            );
        }
        assert_eq!(candidate.state().draws, vec![draw]);
        assert_eq!(candidate.state().decisions.len(), 1);
        assert_eq!(candidate.state().facts.len(), 2);
        assert_eq!(
            candidate.state().decisions[0].source_policy,
            pins.rules.ruleset
        );
        assert!(matches!(
            &candidate.state().facts[1].value,
            FactValue::ContentEvent { definition, .. }
                if *definition == content(expected_entry).unwrap()
        ));
        assert_eq!(current, before);
    }

    let (command, draw) = request(&current, 11);
    let mut other_source = source.clone();
    other_source.clause = label("unsupported-advantage").unwrap();
    for (selector, source, expected) in [
        (
            label("unregistered-generated-handler").unwrap(),
            &source,
            DispatchError::UnknownHandler,
        ),
        (
            selector.clone(),
            &other_source,
            DispatchError::UnsupportedSource,
        ),
    ] {
        assert_eq!(
            registry
                .stage_with_provenance(
                    &pins,
                    &selector,
                    source,
                    RulesCommandInput {
                        command: &command,
                        supplied_draws: std::slice::from_ref(&draw),
                    },
                    &current,
                    1024 * 1024,
                )
                .err(),
            Some(InvocationError::Dispatch(expected))
        );
        assert_eq!(current, before);
    }
}
