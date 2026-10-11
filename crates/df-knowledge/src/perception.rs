//! Borrowed canonical facts filtered by the current observer audience.
use df_model::checkpoint::{
    AudienceScope, Basis, Checkpoint, CheckpointError, CheckpointPins, GameFact,
};
use df_types::MemberId;

/// Supplied by the authenticated projection owner, never a client role flag.
/// A member must still occur in the current checkpoint membership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObserverScope {
    Shared,
    Member(MemberId),
}

/// Finite work and borrowed-output limits, including current membership lookup.
/// Zero permits no work or output in that dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerceptionLimits {
    pub maximum_scan_records: usize,
    pub maximum_member_comparisons: usize,
    pub maximum_selected_facts: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerceptionError {
    Checkpoint(CheckpointError),
    ObserverUnavailable,
    ScanCapacity,
    ComparisonCapacity,
    ResultCapacity,
    AllocationCapacity,
}

/// Facts retain checkpoint order and identity; no truth or disclosure is created.
/// This selection is valid only for this immutable current checkpoint. Its owner
/// must revalidate authorization before later publication or commit. Debug output
/// deliberately omits private fact payloads and observer identities.
pub struct PerceivedFacts<'a> {
    checkpoint: &'a Checkpoint,
    facts: Vec<&'a GameFact>,
}

impl std::fmt::Debug for PerceivedFacts<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PerceivedFacts")
            .field("count", &self.facts.len())
            .finish_non_exhaustive()
    }
}

impl<'a> PerceivedFacts<'a> {
    pub fn basis(&self) -> Basis {
        self.checkpoint.basis()
    }

    pub fn pins(&self) -> &'a CheckpointPins {
        self.checkpoint.pins()
    }

    pub fn facts(&self) -> &[&'a GameFact] {
        &self.facts
    }
}

fn consume(remaining: &mut usize, error: PerceptionError) -> Result<(), PerceptionError> {
    *remaining = remaining.checked_sub(1).ok_or(error)?;
    Ok(())
}

/// Select only Shared facts for a shared observer, and Shared plus explicit
/// Members inclusion for an exact current member. Host facts are always omitted.
/// Knowledge grants, rumors, claims and retained text cannot widen this scope.
/// The trusted owner supplies an exact current basis and admitted pins. Every
/// failure returns no partial selection. Work scans each fact once, and each
/// membership/audience comparison consumes the explicit comparison budget.
pub fn perceive<'a>(
    current: &'a Checkpoint,
    expected: Basis,
    admitted: &CheckpointPins,
    observer: ObserverScope,
    limits: PerceptionLimits,
) -> Result<PerceivedFacts<'a>, PerceptionError> {
    current
        .validate_resume(expected, admitted)
        .map_err(PerceptionError::Checkpoint)?;
    let state = current.state();
    let mut scans = limits.maximum_scan_records;
    let mut comparisons = limits.maximum_member_comparisons;
    if let ObserverScope::Member(member) = observer {
        let mut found = false;
        for link in &state.members {
            consume(&mut scans, PerceptionError::ScanCapacity)?;
            consume(&mut comparisons, PerceptionError::ComparisonCapacity)?;
            if link.member == member {
                found = true;
                break;
            }
        }
        if !found {
            return Err(PerceptionError::ObserverUnavailable);
        }
    }
    // Admit the full fact scan before allocating or selecting anything.
    if state.facts.len() > scans {
        return Err(PerceptionError::ScanCapacity);
    }
    let mut facts = Vec::new();
    facts
        .try_reserve_exact(state.facts.len().min(limits.maximum_selected_facts))
        .map_err(|_| PerceptionError::AllocationCapacity)?;
    for fact in &state.facts {
        let permitted = match (&fact.audience, observer) {
            (AudienceScope::Shared, _) => true,
            (AudienceScope::Members(members), ObserverScope::Member(member)) => {
                let mut included = false;
                for recipient in members {
                    consume(&mut comparisons, PerceptionError::ComparisonCapacity)?;
                    if *recipient == member {
                        included = true;
                        break;
                    }
                }
                included
            }
            (AudienceScope::Members(_), ObserverScope::Shared) | (AudienceScope::Host, _) => false,
        };
        if permitted {
            if facts.len() == limits.maximum_selected_facts {
                return Err(PerceptionError::ResultCapacity);
            }
            facts.push(fact);
        }
    }
    Ok(PerceivedFacts {
        checkpoint: current,
        facts,
    })
}

/// Finite canonical claim, ownership-policy and audience work, plus borrowed output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaimPerceptionLimits {
    pub maximum_scan_records: usize,
    pub maximum_record_comparisons: usize,
    pub maximum_selected_claims: usize,
}

/// Attributed claims remain separate from canonical truth and their evidence facts.
/// This selection carries no reveal or commit authority.
pub struct PerceivedClaims<'a> {
    checkpoint: &'a Checkpoint,
    claims: Vec<&'a df_model::checkpoint::AttributedClaim>,
}

impl std::fmt::Debug for PerceivedClaims<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PerceivedClaims")
            .field("count", &self.claims.len())
            .finish_non_exhaustive()
    }
}

impl<'a> PerceivedClaims<'a> {
    pub fn basis(&self) -> Basis {
        self.checkpoint.basis()
    }

    pub fn pins(&self) -> &'a CheckpointPins {
        self.checkpoint.pins()
    }

    pub fn claims(&self) -> &[&'a df_model::checkpoint::AttributedClaim] {
        &self.claims
    }
}

struct ClaimWork {
    scans: usize,
    comparisons: usize,
}

impl ClaimWork {
    fn visit(&mut self) -> Result<(), PerceptionError> {
        consume(&mut self.scans, PerceptionError::ScanCapacity)?;
        consume(&mut self.comparisons, PerceptionError::ComparisonCapacity)
    }

    fn audience_permits(
        &mut self,
        audience: &AudienceScope,
        observer: ObserverScope,
    ) -> Result<bool, PerceptionError> {
        match (audience, observer) {
            (AudienceScope::Shared, _) => Ok(true),
            (AudienceScope::Members(members), ObserverScope::Member(member)) => {
                for recipient in members {
                    consume(&mut self.comparisons, PerceptionError::ComparisonCapacity)?;
                    if *recipient == member {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            (AudienceScope::Members(_), ObserverScope::Shared) | (AudienceScope::Host, _) => {
                Ok(false)
            }
        }
    }
}

// A policy owns its named claim, not each fact cited as evidence. Invalid or
// competing ownership is suppressed for that claim without disclosing its IDs.
fn secret_claim_permits(
    state: &df_model::checkpoint::GameState,
    claim: &df_model::checkpoint::AttributedClaim,
    observer: ObserverScope,
    work: &mut ClaimWork,
) -> Result<bool, PerceptionError> {
    let mut owner_found = false;
    let mut permitted = true;
    for npc in &state.continuity.npcs {
        work.visit()?;
        for secret in &npc.secrets {
            work.visit()?;
            let mut linked = false;
            for id in &secret.claims {
                work.visit()?;
                if *id == claim.id {
                    linked = true;
                }
            }
            if !linked {
                continue;
            }
            if owner_found || secret.holder != claim.holder || npc.entity != secret.holder {
                permitted = false;
            }
            owner_found = true;
            permitted &= work.audience_permits(&secret.permitted_audience, observer)?;
        }
    }
    Ok(permitted)
}

fn claim_evidence_permits(
    state: &df_model::checkpoint::GameState,
    claim: &df_model::checkpoint::AttributedClaim,
    observer: ObserverScope,
    work: &mut ClaimWork,
) -> Result<bool, PerceptionError> {
    for id in &claim.evidence {
        work.visit()?;
        let mut found = false;
        for fact in &state.facts {
            work.visit()?;
            if fact.id != *id {
                continue;
            }
            found = true;
            if !work.audience_permits(&fact.audience, observer)? {
                return Ok(false);
            }
            break;
        }
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Select claims in checkpoint order under their own explicit audience ceiling.
/// An exact canonical SecretPolicy may only narrow that claim's audience. Its
/// holder must match the claim holder and containing NPC; multiple policies for
/// one claim suppress it until ownership is resolved. Policy references are
/// admitted by canonical checkpoint construction and current content pins.
/// Host claims are omitted; grants, evidence visibility, confidence, relationships
/// and claim text confer no disclosure permission. Evidence facts are never changed
/// or restricted by this operation, including facts cited by false beliefs.
/// Every cited evidence fact must itself be visible to this observer before its
/// identifier may leave in a borrowed claim; a grant cannot widen that audience.
///
/// The authenticated owner supplies the observer, current basis and admitted pins
/// and must revalidate them before later publication. Every refusal returns no
/// partial selection. Ownership scans and audience comparisons share finite work
/// budgets; this function performs no I/O, contact inference, reveal or commit.
pub fn perceive_claims<'a>(
    current: &'a Checkpoint,
    expected: Basis,
    admitted: &CheckpointPins,
    observer: ObserverScope,
    limits: ClaimPerceptionLimits,
) -> Result<PerceivedClaims<'a>, PerceptionError> {
    current
        .validate_resume(expected, admitted)
        .map_err(PerceptionError::Checkpoint)?;
    let state = current.state();
    let mut work = ClaimWork {
        scans: limits.maximum_scan_records,
        comparisons: limits.maximum_record_comparisons,
    };
    if let ObserverScope::Member(member) = observer {
        let mut found = false;
        for link in &state.members {
            work.visit()?;
            if link.member == member {
                found = true;
                break;
            }
        }
        if !found {
            return Err(PerceptionError::ObserverUnavailable);
        }
    }
    if state.beliefs.len() > work.scans {
        return Err(PerceptionError::ScanCapacity);
    }
    let mut claims = Vec::new();
    claims
        .try_reserve_exact(state.beliefs.len().min(limits.maximum_selected_claims))
        .map_err(|_| PerceptionError::AllocationCapacity)?;
    for claim in &state.beliefs {
        work.visit()?;
        if !work.audience_permits(&claim.audience, observer)?
            || !secret_claim_permits(state, claim, observer, &mut work)?
            || !claim_evidence_permits(state, claim, observer, &mut work)?
        {
            continue;
        }
        if claims.len() == limits.maximum_selected_claims {
            return Err(PerceptionError::ResultCapacity);
        }
        claims.push(claim);
    }
    Ok(PerceivedClaims {
        checkpoint: current,
        claims,
    })
}

fn journal_sources_permit(
    state: &df_model::checkpoint::GameState,
    entry: &df_model::checkpoint::JournalEntry,
    observer: ObserverScope,
    work: &mut ClaimWork,
) -> Result<bool, PerceptionError> {
    if entry.source_facts.is_empty() {
        return Ok(false);
    }
    for id in &entry.source_facts {
        work.visit()?;
        let mut found = None;
        for fact in &state.facts {
            work.visit()?;
            if fact.id == *id {
                found = Some(fact);
                break;
            }
        }
        let Some(fact) = found else {
            return Ok(false);
        };
        if !work.audience_permits(&fact.audience, observer)? {
            return Ok(false);
        }
        let mut accepted = false;
        for decision in &state.decisions {
            work.visit()?;
            if decision.operation != fact.operation || decision.revision != fact.revision {
                continue;
            }
            for accepted_id in &decision.facts {
                work.visit()?;
                if *accepted_id == fact.id {
                    accepted = true;
                    break;
                }
            }
            if accepted {
                break;
            }
        }
        if !accepted {
            return Ok(false);
        }
    }
    Ok(true)
}

fn journal_claims_permit(
    state: &df_model::checkpoint::GameState,
    entry: &df_model::checkpoint::JournalEntry,
    observer: ObserverScope,
    work: &mut ClaimWork,
) -> Result<bool, PerceptionError> {
    for id in &entry.attributed_claims {
        work.visit()?;
        let mut found = None;
        for claim in &state.beliefs {
            work.visit()?;
            if claim.id == *id {
                found = Some(claim);
                break;
            }
        }
        let Some(claim) = found else {
            return Ok(false);
        };
        if !work.audience_permits(&claim.audience, observer)?
            || !secret_claim_permits(state, claim, observer, work)?
            || !claim_evidence_permits(state, claim, observer, work)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Borrow source-qualified journal entries in checkpoint order. Every cited fact
/// must be visible and bound by its exact retained decision operation/revision;
/// every attributed claim retains its own audience, secret-policy and evidence
/// checks. Source-free notes remain stored but are omitted from this selection.
///
/// The authenticated owner supplies the current checkpoint, exact basis, pins and
/// observer. A retained decision binding does not prove durable Session commit:
/// the owner must supply authoritative committed state before publication. This
/// function performs no I/O and never promotes a claim or text into a fact.
/// Nested record scans and comparisons share the supplied finite work budgets.
/// Zero permits no work/output in that dimension; every error discards all output.
pub fn perceive_journal<'a>(
    current: &'a Checkpoint,
    expected: Basis,
    admitted: &CheckpointPins,
    observer: ObserverScope,
    maximum_scan_records: usize,
    maximum_record_comparisons: usize,
    maximum_selected_entries: usize,
) -> Result<Vec<&'a df_model::checkpoint::JournalEntry>, PerceptionError> {
    current
        .validate_resume(expected, admitted)
        .map_err(PerceptionError::Checkpoint)?;
    let state = current.state();
    let mut work = ClaimWork {
        scans: maximum_scan_records,
        comparisons: maximum_record_comparisons,
    };
    if let ObserverScope::Member(member) = observer {
        let mut found = false;
        for link in &state.members {
            work.visit()?;
            if link.member == member {
                found = true;
                break;
            }
        }
        if !found {
            return Err(PerceptionError::ObserverUnavailable);
        }
    }
    if state.continuity.journal.len() > work.scans {
        return Err(PerceptionError::ScanCapacity);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(state.continuity.journal.len().min(maximum_selected_entries))
        .map_err(|_| PerceptionError::AllocationCapacity)?;
    for entry in &state.continuity.journal {
        work.visit()?;
        if !work.audience_permits(&entry.audience, observer)?
            || !journal_sources_permit(state, entry, observer, &mut work)?
            || !journal_claims_permit(state, entry, observer, &mut work)?
        {
            continue;
        }
        if entries.len() == maximum_selected_entries {
            return Err(PerceptionError::ResultCapacity);
        }
        entries.push(entry);
    }
    Ok(entries)
}
