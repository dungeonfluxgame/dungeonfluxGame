//! Borrowed dispatch selection over an owner-admitted, pinned source inventory.
//! This structural boundary does not qualify sources, rights, or mechanics.
use crate::explanation::RuleExplanation;
use df_content::catalog::CatalogSnapshot;
use df_model::checkpoint::{CheckpointPins, RuleReference};
use df_types::RevisionLabel;

/// One compiled handler value associated with a reviewed selector and source clause.
/// The handler type belongs to the canonical rules owner; imported bytes are never executed.
pub struct HandlerRegistration<'a, Handler> {
    selector: &'a RevisionLabel,
    source: &'a RuleReference,
    handler: &'a Handler,
}

impl<'a, Handler> HandlerRegistration<'a, Handler> {
    /// Pairs a reviewed handler with the selector and source clause it is registered under.
    pub fn new(
        selector: &'a RevisionLabel,
        source: &'a RuleReference,
        handler: &'a Handler,
    ) -> Self {
        Self {
            selector,
            source,
            handler,
        }
    }
}

/// Safe structural admission refusals; no error retains imported source bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    RegistrationLimit {
        actual: usize,
        maximum: usize,
    },
    CatalogVersionMismatch,
    SourceCatalogMismatch {
        registration_index: usize,
    },
    MissingSource {
        registration_index: usize,
    },
    DuplicateRegistration {
        first_index: usize,
        duplicate_index: usize,
    },
}

/// Refusals preserve missing handlers and unsupported clauses as explicit gaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchError {
    PinsMismatch,
    UnknownHandler,
    UnsupportedSource,
}

/// Immutable bounded mapping of exact source clauses to already-reviewed typed handlers.
///
/// The content owner supplies a decoded, source-qualified inventory bound to the complete
/// checkpoint pins. The catalog's structural validation does not establish that qualification.
/// The rules owner supplies the closed compiled handler type and registrations; this registry
/// does not interpret or execute content bytes and provides no latest-version lookup or fallback.
/// Selection does not execute mechanics, draw dice, mutate state, or grant source-use rights.
pub struct DispatchRegistry<'a, Handler> {
    catalog: CatalogSnapshot<'a, RevisionLabel, CheckpointPins, RuleReference>,
    registrations: &'a [HandlerRegistration<'a, Handler>],
}

impl<'a, Handler> DispatchRegistry<'a, Handler> {
    /// Validates the entire registration set before exposing a handler.
    /// Comparisons are quadratic in the explicit registration limit, with no allocation.
    /// The supplied catalog separately enforces its own entry and byte limits.
    pub fn from_catalog(
        catalog: CatalogSnapshot<'a, RevisionLabel, CheckpointPins, RuleReference>,
        registrations: &'a [HandlerRegistration<'a, Handler>],
        max_registrations: usize,
    ) -> Result<Self, RegistryError> {
        if registrations.len() > max_registrations {
            return Err(RegistryError::RegistrationLimit {
                actual: registrations.len(),
                maximum: max_registrations,
            });
        }
        if catalog.version() != &catalog.pins().rules.catalog {
            return Err(RegistryError::CatalogVersionMismatch);
        }
        for (registration_index, registration) in registrations.iter().enumerate() {
            if registration.source.catalog != catalog.pins().rules.catalog {
                return Err(RegistryError::SourceCatalogMismatch { registration_index });
            }
            match catalog.get(catalog.version(), registration.source) {
                Ok(Some(_)) => {}
                Ok(None) => return Err(RegistryError::MissingSource { registration_index }),
                Err(_) => return Err(RegistryError::CatalogVersionMismatch),
            }
            for (first_index, prior) in registrations.iter().take(registration_index).enumerate() {
                if prior.selector == registration.selector && prior.source == registration.source {
                    return Err(RegistryError::DuplicateRegistration {
                        first_index,
                        duplicate_index: registration_index,
                    });
                }
            }
        }
        Ok(Self {
            catalog,
            registrations,
        })
    }

    /// Returns the exact registered handler only for the caller's complete pinned basis.
    /// A known selector cannot execute a different clause or edition through this boundary.
    pub fn select(
        &self,
        expected_pins: &CheckpointPins,
        selector: &RevisionLabel,
        source: &RuleReference,
    ) -> Result<&'a Handler, DispatchError> {
        Ok(self.registration(expected_pins, selector, source)?.handler)
    }

    /// Selects the same compiled registration as select and explains its exact supplied basis.
    /// No source bytes are executed or copied; missing/unsupported selections return no view.
    pub fn select_with_provenance(
        &self,
        expected_pins: &CheckpointPins,
        selector: &RevisionLabel,
        source: &RuleReference,
    ) -> Result<(&'a Handler, RuleExplanation<'a>), DispatchError> {
        let registration = self.registration(expected_pins, selector, source)?;
        let source_bytes = self
            .catalog
            .get(self.catalog.version(), registration.source)
            .map_err(|_| DispatchError::PinsMismatch)?
            .ok_or(DispatchError::UnsupportedSource)?;
        Ok((
            registration.handler,
            RuleExplanation {
                pins: self.catalog.pins(),
                selector: registration.selector,
                source: registration.source,
                source_bytes,
            },
        ))
    }

    fn registration(
        &self,
        expected_pins: &CheckpointPins,
        selector: &RevisionLabel,
        source: &RuleReference,
    ) -> Result<&'a HandlerRegistration<'a, Handler>, DispatchError> {
        if expected_pins != self.catalog.pins() {
            return Err(DispatchError::PinsMismatch);
        }
        let mut known_selector = false;
        for registration in self.registrations {
            if registration.selector == selector {
                known_selector = true;
                if registration.source == source {
                    return Ok(registration);
                }
            }
        }
        if known_selector {
            Err(DispatchError::UnsupportedSource)
        } else {
            Err(DispatchError::UnknownHandler)
        }
    }
}
