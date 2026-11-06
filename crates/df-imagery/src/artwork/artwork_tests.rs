use super::*;
use df_types::AccountId;

fn campaign(byte: u8) -> CampaignId {
    CampaignId::from_bytes(&[byte; 16]).unwrap()
}

fn run(byte: u8) -> RunId {
    RunId::from_bytes(&[byte; 16]).unwrap()
}

fn person_spec() -> CampaignArtworkSpec {
    CampaignArtworkSpec {
        campaign: campaign(1),
        subject: ArtworkSubject::Hero(ArtworkSubjectKey::new("library/public-hero").unwrap()),
        public_revision: [2; 32],
    }
}

fn binding() -> ArtworkBinding {
    ArtworkBinding {
        spec: person_spec(),
        selection: RunSelectionFence {
            run: run(3),
            binding: [4; 32],
        },
    }
}

fn purpose() -> PurposedUse {
    PurposedUse {
        purpose: Purpose::CanonicalPerson,
        role: VariantRole::Primary,
    }
}

#[test]
fn different_run_selection_fences_keep_the_same_public_prompt_binding() {
    let first = binding();
    let mut next_run = first.clone();
    next_run.selection.run = run(5);
    next_run.selection.binding = [6; 32];
    assert_ne!(first, next_run);
    assert_eq!(first.spec.binding_digest(), next_run.spec.binding_digest());
    for artwork in [first, next_run] {
        assert_eq!(
            artwork.validate_for(
                &ImageOwner::Run {
                    campaign: campaign(1),
                    run: artwork.selection.run,
                },
                purpose(),
                &Audience::CampaignShared(campaign(1)),
            ),
            Ok(())
        );
    }
}

#[test]
fn subject_namespace_campaign_and_public_revision_each_change_the_prompt_binding() {
    let original = person_spec();
    let mut seen = std::collections::BTreeSet::new();
    for subject in [
        ArtworkSubject::Hero(ArtworkSubjectKey::new("same-id").unwrap()),
        ArtworkSubject::Npc(ArtworkSubjectKey::new("same-id").unwrap()),
        ArtworkSubject::Cast(ArtworkSubjectKey::new("same-id").unwrap()),
        ArtworkSubject::Item(ArtworkSubjectKey::new("same-id").unwrap()),
    ] {
        let mut spec = original.clone();
        spec.subject = subject;
        assert!(seen.insert(spec.binding_digest()));
    }
    let mut changed = original.clone();
    changed.campaign = campaign(7);
    assert_ne!(original.binding_digest(), changed.binding_digest());
    changed = original.clone();
    changed.public_revision = [8; 32];
    assert_ne!(original.binding_digest(), changed.binding_digest());
}

#[test]
fn subject_keys_are_byte_bounded_exact_and_redacted() {
    assert_eq!(ArtworkSubjectKey::new(""), Err(ArtworkError::EmptySubject));
    assert_eq!(
        ArtworkSubjectKey::new("   "),
        Err(ArtworkError::EmptySubject)
    );
    assert!(ArtworkSubjectKey::new(&"x".repeat(MAXIMUM_ARTWORK_SUBJECT_BYTES)).is_ok());
    assert_eq!(
        ArtworkSubjectKey::new(&"x".repeat(MAXIMUM_ARTWORK_SUBJECT_BYTES + 1)),
        Err(ArtworkError::SubjectTooLong {
            actual_bytes: MAXIMUM_ARTWORK_SUBJECT_BYTES + 1,
        })
    );
    let spaced = ArtworkSubjectKey::new(" public definition ").unwrap();
    assert_eq!(spaced.as_str(), " public definition ");
    assert_ne!(spaced, ArtworkSubjectKey::new("public definition").unwrap());
    let debug = format!("{spaced:?}");
    assert!(!debug.contains("public definition"));
    assert!(debug.contains("len_bytes"));
}

#[test]
fn draft_other_campaign_and_other_run_claims_never_pass_artwork_scope_validation() {
    let artwork = binding();
    for owner in [
        ImageOwner::Draft {
            account: AccountId::from_bytes(&[1; 16]).unwrap(),
        },
        ImageOwner::Run {
            campaign: campaign(9),
            run: run(3),
        },
        ImageOwner::Run {
            campaign: campaign(1),
            run: run(9),
        },
    ] {
        assert_eq!(
            artwork.validate_for(&owner, purpose(), &Audience::CampaignShared(campaign(1))),
            Err(ArtworkError::OwnerScopeMismatch)
        );
    }
}

#[test]
fn private_audience_and_noncanonical_purpose_or_role_never_pass_artwork_validation() {
    let artwork = binding();
    let owner = ImageOwner::Run {
        campaign: campaign(1),
        run: run(3),
    };
    assert_eq!(
        artwork.validate_for(&owner, purpose(), &Audience::PublicStatic),
        Err(ArtworkError::AudienceMismatch)
    );
    for purpose in [
        PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        PurposedUse {
            purpose: Purpose::CanonicalPerson,
            role: VariantRole::Reference,
        },
        PurposedUse {
            purpose: Purpose::CreatorPortrait,
            role: VariantRole::Primary,
        },
    ] {
        assert_eq!(
            artwork.validate_for(&owner, purpose, &Audience::CampaignShared(campaign(1))),
            Err(ArtworkError::PurposeMismatch)
        );
    }
}

#[test]
fn item_identity_requires_the_item_close_up_purpose() {
    let mut artwork = binding();
    artwork.spec.subject = ArtworkSubject::Item(ArtworkSubjectKey::new("scene/thing").unwrap());
    assert_eq!(
        artwork.validate_for(
            &ImageOwner::Run {
                campaign: campaign(1),
                run: run(3),
            },
            PurposedUse {
                purpose: Purpose::ItemCloseUp,
                role: VariantRole::Primary,
            },
            &Audience::CampaignShared(campaign(1)),
        ),
        Ok(())
    );
}
