use crate::composition_registry::*;

const CONSUMER_SOURCES: &[(&str, &str)] = &[
    (
        "crates/df-session/src/inbox.rs",
        include_str!("../../df-session/src/inbox.rs"),
    ),
    (
        "crates/df-session/src/submission.rs",
        include_str!("../../df-session/src/submission.rs"),
    ),
    (
        "crates/df-session/src/effects.rs",
        include_str!("../../df-session/src/effects.rs"),
    ),
    (
        "crates/df-engine/src/effect_emission.rs",
        include_str!("../../df-engine/src/effect_emission.rs"),
    ),
    (
        "crates/df-assets/src/publication.rs",
        include_str!("../../df-assets/src/publication.rs"),
    ),
    (
        "crates/df-assets/src/range.rs",
        include_str!("../../df-assets/src/range.rs"),
    ),
    (
        "crates/df-auth/src/bootstrap.rs",
        include_str!("../../df-auth/src/bootstrap.rs"),
    ),
    (
        "crates/df-auth/src/membership.rs",
        include_str!("../../df-auth/src/membership.rs"),
    ),
    (
        "crates/df-auth/src/permissions.rs",
        include_str!("../../df-auth/src/permissions.rs"),
    ),
    (
        "crates/df-auth/src/recovery.rs",
        include_str!("../../df-auth/src/recovery.rs"),
    ),
    (
        "crates/df-auth/src/rotation.rs",
        include_str!("../../df-auth/src/rotation.rs"),
    ),
    (
        "crates/df-observe/src/ingress.rs",
        include_str!("../../df-observe/src/ingress.rs"),
    ),
    (
        "crates/df-ai/src/admission.rs",
        include_str!("../../df-ai/src/admission.rs"),
    ),
    (
        "crates/df-ai/src/lookup.rs",
        include_str!("../../df-ai/src/lookup.rs"),
    ),
    (
        "crates/df-provider-api/src/budget.rs",
        include_str!("../../df-provider-api/src/budget.rs"),
    ),
];
const BRIDGE_SOURCE: &str = include_str!("../../df-rpc-bridge/src/native.rs");
const MODEL_SOURCE: &str = include_str!("../../df-model/src/checkpoint.rs");

fn assignments() -> Vec<OwnershipAssignment> {
    NATIVE_BOUNDARIES
        .iter()
        .map(|spec| OwnershipAssignment {
            boundary: spec.key,
            contract_owner: Some(spec.contract_owner),
            lifetime_owner: Some(spec.lifetime_owner),
        })
        .collect()
}

#[test]
fn complete_registry_covers_actual_source_ports_effects_and_owned_native_surfaces() {
    let supplied = assignments();
    let registry = OwnershipRegistry::validate(&supplied).unwrap();
    let mut declared_ports = 0;
    for (path, text) in CONSUMER_SOURCES {
        for line in text
            .lines()
            .filter_map(|line| line.strip_prefix("pub trait "))
        {
            let symbol = line
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .next()
                .unwrap();
            let matching: Vec<_> = NATIVE_BOUNDARIES
                .iter()
                .filter(|spec| {
                    spec.key.kind == BoundaryKind::Consumer
                        && spec.source_path == *path
                        && spec.source_symbol == symbol
                })
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "current public port {path}::{symbol} must have one owner"
            );
            let spec = matching[0];
            let source_owner = path.split('/').nth(1).unwrap();
            assert_eq!(spec.contract_owner.crate_name(), source_owner);
            assert_eq!(
                registry
                    .owner(spec.key)
                    .unwrap()
                    .contract_owner
                    .unwrap()
                    .crate_name(),
                source_owner
            );
            declared_ports += 1;
        }
    }
    assert_eq!(
        declared_ports,
        NATIVE_BOUNDARIES
            .iter()
            .filter(|s| s.key.kind == BoundaryKind::Consumer)
            .count()
    );
    let kinds = MODEL_SOURCE
        .split("pub enum EffectKind {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    let mut effects = 0;
    for line in kinds.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let kind = line.strip_suffix(',').unwrap();
        let spec = NATIVE_BOUNDARIES
            .iter()
            .find(|spec| spec.key.kind == BoundaryKind::Executor && spec.source_symbol == kind)
            .unwrap();
        assert_eq!(
            registry.owner(spec.key).unwrap().lifetime_owner,
            Some(CrateOwner::Server)
        );
        assert_eq!(
            registry.native_admission(spec.key),
            Err(NativeRefusal::MissingEffectExecutor(spec.key))
        );
        effects += 1;
    }
    assert_eq!(effects, 7);
    for symbol in [
        "NativeIncoming",
        "NativeAdmission",
        "NativeConnectionPermit",
    ] {
        assert!(
            BRIDGE_SOURCE
                .lines()
                .any(|line| line.starts_with(&format!("pub struct {symbol} {{")))
        );
        let spec = NATIVE_BOUNDARIES
            .iter()
            .find(|spec| spec.key.kind == BoundaryKind::Bridge && spec.source_symbol == symbol)
            .unwrap();
        assert_eq!(spec.contract_owner, CrateOwner::RpcBridge);
        assert_eq!(spec.source_path, "crates/df-rpc-bridge/src/native.rs");
        assert_eq!(
            registry.native_admission(spec.key),
            Err(NativeRefusal::RuntimeQualificationUnperformed(spec.key))
        );
    }
    assert_eq!(
        NATIVE_BOUNDARIES
            .iter()
            .filter(|spec| spec.key.kind == BoundaryKind::Bridge)
            .count(),
        3
    );
    for spec in NATIVE_BOUNDARIES {
        assert_eq!(
            registry.owner(spec.key).unwrap().lifetime_owner,
            Some(spec.lifetime_owner)
        );
        assert!(registry.native_admission(spec.key).is_err());
    }
    println!(
        "ownership witness: {declared_ports} actual consumer ports, {effects} canonical effect routes, 3 native backing declarations, 3 native bridge contracts, 2 fixture listeners, 3 unimplemented production listeners; runtime admission refused"
    );
}

#[test]
fn every_missing_boundary_and_each_missing_owner_refuses_the_whole_registry() {
    let normal = assignments();
    for (index, entry) in normal.iter().enumerate() {
        let mut omitted = normal.clone();
        omitted.remove(index);
        assert_eq!(
            OwnershipRegistry::validate(&omitted).unwrap_err(),
            OwnershipRefusal::MissingBoundary(entry.boundary)
        );
        for role in [OwnerRole::Contract, OwnerRole::Lifetime] {
            let mut missing = normal.clone();
            match role {
                OwnerRole::Contract => missing[index].contract_owner = None,
                OwnerRole::Lifetime => missing[index].lifetime_owner = None,
            }
            assert_eq!(
                OwnershipRegistry::validate(&missing).unwrap_err(),
                OwnershipRefusal::MissingOwner {
                    boundary: entry.boundary,
                    role
                }
            );
        }
    }
    assert!(matches!(
        OwnershipRegistry::validate(&[]),
        Err(OwnershipRefusal::MissingBoundary(_))
    ));
}

#[test]
fn duplicate_wrong_unknown_and_over_capacity_registrations_cannot_mint_ownership() {
    let normal = assignments();
    for (index, entry) in normal.iter().enumerate() {
        let mut duplicate = normal.clone();
        let target = (index + 1) % normal.len();
        duplicate[target] = *entry;
        assert!(
            matches!(OwnershipRegistry::validate(&duplicate), Err(OwnershipRefusal::DuplicateBoundary(key)) if key == entry.boundary)
        );
        for role in [OwnerRole::Contract, OwnerRole::Lifetime] {
            let mut wrong = normal.clone();
            let expected = match role {
                OwnerRole::Contract => entry.contract_owner.unwrap(),
                OwnerRole::Lifetime => entry.lifetime_owner.unwrap(),
            };
            let observed = if expected == CrateOwner::Server {
                CrateOwner::Tools
            } else {
                CrateOwner::Server
            };
            match role {
                OwnerRole::Contract => wrong[index].contract_owner = Some(observed),
                OwnerRole::Lifetime => wrong[index].lifetime_owner = Some(observed),
            }
            assert_eq!(
                OwnershipRegistry::validate(&wrong).unwrap_err(),
                OwnershipRefusal::WrongOwner {
                    boundary: entry.boundary,
                    role,
                    expected,
                    observed
                }
            );
        }
    }
    let mut unknown = normal.clone();
    let key = BoundaryKey {
        kind: BoundaryKind::Consumer,
        name: "unregistered port",
    };
    unknown[0].boundary = key;
    assert_eq!(
        OwnershipRegistry::validate(&unknown).unwrap_err(),
        OwnershipRefusal::UnknownBoundary(key)
    );
    let mut oversize = normal.clone();
    oversize.push(normal[0]);
    assert_eq!(
        OwnershipRegistry::validate(&oversize).unwrap_err(),
        OwnershipRefusal::Capacity
    );
    let registry = OwnershipRegistry::validate(&normal).unwrap();
    assert_eq!(
        registry.owner(key),
        Err(OwnershipRefusal::UnknownBoundary(key))
    );
}

#[test]
fn adapter_and_listener_ownership_never_establish_native_readiness() {
    let normal = assignments();
    let registry = OwnershipRegistry::validate(&normal).unwrap();
    for spec in NATIVE_BOUNDARIES {
        let expected = match spec.evidence {
            SourceEvidence::MissingEffectExecutor => NativeRefusal::MissingEffectExecutor(spec.key),
            SourceEvidence::FixtureListenerOnly => NativeRefusal::FixtureListenerOnly(spec.key),
            SourceEvidence::MissingProductionListener => {
                NativeRefusal::MissingProductionListener(spec.key)
            }
            SourceEvidence::ConsumerContract
            | SourceEvidence::NativeAdapterDeclaration
            | SourceEvidence::NativeBridgeContract => {
                NativeRefusal::RuntimeQualificationUnperformed(spec.key)
            }
        };
        assert_eq!(registry.native_admission(spec.key), Err(expected));
    }
}
