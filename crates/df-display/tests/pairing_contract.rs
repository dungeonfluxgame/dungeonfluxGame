#![cfg(not(target_arch = "wasm32"))]

use std::{cell::Cell, rc::Rc};

use df_auth::rotation::{
    CredentialFence, CurrentPublicationState, PublicationAuthority, PublicationError,
    PublicationLease, PublicationRefusal, PublicationScope,
};
use df_display::{DisplayPairingError, DisplayPairingMode, publish_paired_display};
use df_model::checkpoint::AudienceScope;
use df_types::{ClientBindingId, MemberId, SessionId};
use df_ui::{
    CharacterDisplayConnection, CharacterDisplayHostOffer, CharacterDisplayLimits,
    CharacterDisplayView, CharacterPublicReadiness,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceError {
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConsumerError {
    Unavailable,
}

// Controlled native authority fixture, not a production role/grant issuer. It
// implements the actual current auth fence; no display request supplies its facts.
struct Authority {
    scope: PublicationScope<u64, u64, AudienceScope>,
    credential: CredentialFence<u64>,
    binding_generation: u64,
    audience_version: u64,
    binding_active: bool,
    credential_unexpired: bool,
    binding_unexpired: bool,
    audience_authorized: bool,
    unavailable: bool,
    missing: bool,
    reads: usize,
    fenced: Rc<Cell<bool>>,
}
impl PublicationAuthority for Authority {
    type Principal = u64;
    type Credential = u64;
    type Audience = AudienceScope;
    type CredentialGeneration = u64;
    type BindingGeneration = u64;
    type AudienceVersion = u64;
    type Error = SourceError;

    fn with_current<R>(
        &mut self,
        _scope: &PublicationScope<u64, u64, AudienceScope>,
        publish: impl FnOnce(Option<CurrentPublicationState<'_, Self>>) -> R,
    ) -> Result<R, SourceError> {
        self.reads += 1;
        if self.unavailable {
            return Err(SourceError::Unavailable);
        }
        self.fenced.set(true);
        let result = if self.missing {
            publish(None)
        } else {
            publish(Some(CurrentPublicationState {
                scope: &self.scope,
                credential: &self.credential,
                binding_generation: &self.binding_generation,
                binding_active: self.binding_active,
                credential_unexpired: self.credential_unexpired,
                binding_unexpired: self.binding_unexpired,
                audience_version: &self.audience_version,
                audience_authorized: self.audience_authorized,
            }))
        };
        self.fenced.set(false);
        Ok(result)
    }
}
fn scope(audience: AudienceScope) -> PublicationScope<u64, u64, AudienceScope> {
    PublicationScope {
        audience,
        principal: 7,
        credential: 11,
        session: SessionId::from_bytes(&[1; 16]).expect("fixture session"),
        binding: ClientBindingId::from_bytes(&[2; 16]).expect("fixture binding"),
    }
}
fn authority(audience: AudienceScope) -> Authority {
    Authority {
        scope: scope(audience),
        credential: CredentialFence::new(3),
        binding_generation: 5,
        audience_version: 9,
        binding_active: true,
        credential_unexpired: true,
        binding_unexpired: true,
        audience_authorized: true,
        unavailable: false,
        missing: false,
        reads: 0,
        fenced: Rc::new(Cell::new(false)),
    }
}
fn lease(audience: AudienceScope) -> PublicationLease<Authority> {
    PublicationLease::new(scope(audience), 3, 5, 9)
}
fn offers() -> Vec<CharacterDisplayHostOffer> {
    vec![CharacterDisplayHostOffer {
        id: "current-host-offer".into(),
        label: "Configured host control".into(),
        enabled: true,
        pending: false,
    }]
}
fn presentation(
    mode: DisplayPairingMode,
    advertised: &[CharacterDisplayHostOffer],
) -> CharacterDisplayView {
    let host_offers = match mode {
        DisplayPairingMode::Passive => Vec::new(),
        DisplayPairingMode::HostControls => advertised.to_vec(),
    };
    let view = CharacterDisplayView {
        generation: 5,
        public_scope_key: "public-fixture".into(),
        revision: 13,
        chapter: "Chapter".into(),
        title: "Public party".into(),
        description: "Public readiness".into(),
        readiness: CharacterPublicReadiness::Ready,
        progress_label: "Ready".into(),
        public_notice: String::new(),
        connection: CharacterDisplayConnection::Connected,
        connection_label: "Connected".into(),
        members: Vec::new(),
        host_offers,
    };
    view.validate(CharacterDisplayLimits {
        max_members: 4,
        max_host_offers: 4,
        max_text_bytes: 512,
    })
    .expect("actual UI contract accepts public fixture");
    view
}
fn refusal(source: &mut Authority, lease: &PublicationLease<Authority>) -> PublicationRefusal {
    let calls = Cell::new(0);
    let result = publish_paired_display(source, lease, |_| {
        calls.set(calls.get() + 1);
        Ok::<_, ConsumerError>(())
    });
    assert_eq!(calls.get(), 0);
    assert!(!source.fenced.get());
    match result {
        Err(DisplayPairingError::Publication(PublicationError::Refused(reason))) => reason,
        other => panic!("expected exact native refusal, got {other:?}"),
    }
}

#[test]
fn passive_large_screen_has_no_host_offers() {
    for device in ["TV", "large laptop", "small phone"] {
        let mut source = authority(AudienceScope::Shared);
        let supplied = offers();
        let fence = Rc::clone(&source.fenced);
        let view = publish_paired_display(&mut source, &lease(AudienceScope::Shared), |mode| {
            assert!(fence.get(), "{device} callback must remain fenced");
            assert_eq!(mode, DisplayPairingMode::Passive);
            Ok::<_, ConsumerError>(presentation(mode, &supplied))
        })
        .unwrap();
        assert!(view.host_offers.is_empty());
        assert_eq!(source.reads, 1);
        assert_eq!(supplied, offers());
        assert!(!source.fenced.get());
    }
}

#[test]
fn invitation_and_requested_role_cannot_promote() {
    // Matching Host audience labels alone cannot override the auth owner's denial.
    let mut source = authority(AudienceScope::Host);
    source.audience_authorized = false;
    assert_eq!(
        refusal(&mut source, &lease(AudienceScope::Host)),
        PublicationRefusal::AudienceDenied
    );
    // A public display lease also cannot substitute for a Host audience lease.
    let mut source = authority(AudienceScope::Shared);
    assert_eq!(
        refusal(&mut source, &lease(AudienceScope::Host)),
        PublicationRefusal::ScopeChanged
    );
}

#[test]
fn explicit_current_host_has_only_advertised_controls() {
    let mut source = authority(AudienceScope::Host);
    let supplied = offers();
    let fence = Rc::clone(&source.fenced);
    let view = publish_paired_display(&mut source, &lease(AudienceScope::Host), |mode| {
        assert!(fence.get());
        assert_eq!(mode, DisplayPairingMode::HostControls);
        Ok::<_, ConsumerError>(presentation(mode, &supplied))
    })
    .unwrap();
    assert_eq!(view.host_offers, supplied);
    assert_eq!(source.reads, 1);
    assert!(!source.fenced.get());
}

#[test]
fn member_private_audience_is_rejected() {
    let member = MemberId::from_bytes(&[4; 16]).expect("fixture member");
    for audience in [
        AudienceScope::Members(Vec::new()),
        AudienceScope::Members(vec![member]),
    ] {
        let mut source = authority(audience.clone());
        let calls = Cell::new(0);
        let result = publish_paired_display(&mut source, &lease(audience), |_| {
            calls.set(calls.get() + 1);
            Ok::<_, ConsumerError>(())
        });
        assert_eq!(result, Err(DisplayPairingError::PrivateAudience));
        assert_eq!(calls.get(), 0);
        assert_eq!(source.reads, 0);
    }
}

#[test]
fn scope_mismatch_and_replacement_refuse() {
    let changes: [fn(&mut Authority); 4] = [
        |authority| authority.scope.session = SessionId::from_bytes(&[6; 16]).unwrap(),
        |authority| authority.scope.binding = ClientBindingId::from_bytes(&[6; 16]).unwrap(),
        |authority| authority.scope.principal += 1,
        |authority| authority.scope.credential += 1,
    ];
    for change in changes {
        let mut source = authority(AudienceScope::Host);
        change(&mut source);
        assert_eq!(
            refusal(&mut source, &lease(AudienceScope::Host)),
            PublicationRefusal::ScopeChanged
        );
    }
    let mut source = authority(AudienceScope::Host);
    source.binding_generation += 1;
    assert_eq!(
        refusal(&mut source, &lease(AudienceScope::Host)),
        PublicationRefusal::BindingReplaced
    );
}

#[test]
fn revocation_rotation_and_expiry_refuse() {
    type RefusalCase = (fn(&mut Authority), PublicationRefusal);
    let cases: [RefusalCase; 5] = [
        (
            |authority| authority.credential.revoke(),
            PublicationRefusal::CredentialRevoked,
        ),
        (
            |authority| authority.credential.rotate(4).unwrap(),
            PublicationRefusal::CredentialRotated,
        ),
        (
            |authority| authority.credential_unexpired = false,
            PublicationRefusal::CredentialExpired,
        ),
        (
            |authority| authority.binding_active = false,
            PublicationRefusal::BindingRevoked,
        ),
        (
            |authority| authority.binding_unexpired = false,
            PublicationRefusal::BindingExpired,
        ),
    ];
    for (change, expected) in cases {
        let mut source = authority(AudienceScope::Host);
        let retained = publish_paired_display(&mut source, &lease(AudienceScope::Host), |mode| {
            Ok::<_, ConsumerError>(presentation(mode, &offers()))
        })
        .unwrap();
        let before = retained.clone();
        change(&mut source);
        assert_eq!(refusal(&mut source, &lease(AudienceScope::Host)), expected);
        assert_eq!(retained, before);
    }
}

#[test]
fn host_loss_or_audience_version_change_refuse() {
    for loss in [true, false] {
        let mut source = authority(AudienceScope::Host);
        let retained_lease = lease(AudienceScope::Host);
        let retained = publish_paired_display(&mut source, &retained_lease, |mode| {
            Ok::<_, ConsumerError>(presentation(mode, &offers()))
        })
        .unwrap();
        let expected = if loss {
            source.audience_authorized = false;
            PublicationRefusal::AudienceDenied
        } else {
            source.audience_version += 1;
            PublicationRefusal::AudienceChanged
        };
        assert_eq!(refusal(&mut source, &retained_lease), expected);
        assert_eq!(retained.host_offers, offers());
        assert_eq!(source.reads, 2);
    }
}

#[test]
fn authority_and_consumer_failure_preserved() {
    let mut source = authority(AudienceScope::Shared);
    source.unavailable = true;
    let result = publish_paired_display::<_, (), ConsumerError>(
        &mut source,
        &lease(AudienceScope::Shared),
        |_| {
            panic!("source failure must not invoke producer");
        },
    );
    assert_eq!(
        result,
        Err(DisplayPairingError::Publication(
            PublicationError::Authority(SourceError::Unavailable)
        ))
    );
    source.unavailable = false;
    source.missing = true;
    assert_eq!(
        refusal(&mut source, &lease(AudienceScope::Shared)),
        PublicationRefusal::MissingAuthority
    );
    source.missing = false;
    let calls = Cell::new(0);
    let fence = Rc::clone(&source.fenced);
    let result = publish_paired_display(&mut source, &lease(AudienceScope::Shared), |_| {
        assert!(fence.get());
        calls.set(calls.get() + 1);
        Err::<(), _>(ConsumerError::Unavailable)
    });
    assert_eq!(calls.get(), 1);
    assert_eq!(
        result,
        Err(DisplayPairingError::Publication(
            PublicationError::Publisher(ConsumerError::Unavailable)
        ))
    );
    assert!(!source.fenced.get());
}
