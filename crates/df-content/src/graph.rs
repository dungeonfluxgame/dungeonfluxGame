use std::collections::{BTreeMap, BTreeSet};

/// Borrowed authored progression. Beat identity remains owned by the caller.
#[derive(Debug)]
pub struct GraphBeat<'a, BeatId> {
    pub id: &'a BeatId,
    pub next: &'a [BeatId],
    pub terminal: bool,
}

/// A structural route; its optional fact reference grants no disclosure permission.
#[derive(Debug)]
pub struct GraphAlternative<'a, BeatId, FactId, AlternativeId> {
    pub id: &'a AlternativeId,
    pub owner: &'a BeatId,
    pub entry: &'a BeatId,
    pub exit: &'a BeatId,
    pub disclosure_fact: Option<&'a FactId>,
}

/// Supplied immutable graph data, without runtime conditions, schema, or publication authority.
#[derive(Debug)]
pub struct GraphView<'a, BeatId, FactId, AlternativeId> {
    pub start: &'a BeatId,
    pub beats: &'a [GraphBeat<'a, BeatId>],
    pub facts: &'a [FactId],
    pub alternatives: &'a [GraphAlternative<'a, BeatId, FactId, AlternativeId>],
}

/// Caller-selected bounds. Zero grants zero capacity; limits never truncate accepted input.
/// Work counts input records, traversed nodes/edges, and initialized traversal slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphLimits {
    pub max_beats: usize,
    pub max_edges: usize,
    pub max_facts: usize,
    pub max_alternatives: usize,
    pub max_work: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphLimit {
    Beats,
    Edges,
    Facts,
    Alternatives,
    Work,
}

/// First refusal in supplied record/edge order. IDs retain their caller-owned types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError<BeatId, FactId, AlternativeId> {
    LimitExceeded(GraphLimit),
    DuplicateBeat(BeatId),
    DuplicateFact(FactId),
    DuplicateAlternative(AlternativeId),
    MissingStart(BeatId),
    MissingEdge {
        from: BeatId,
        to: BeatId,
    },
    Cycle {
        from: BeatId,
        to: BeatId,
    },
    NoReachableExit,
    MissingAlternativeOwner {
        alternative: AlternativeId,
        owner: BeatId,
    },
    MissingAlternativeEntry {
        alternative: AlternativeId,
        entry: BeatId,
    },
    MissingAlternativeExit {
        alternative: AlternativeId,
        exit: BeatId,
    },
    MissingDisclosureFact {
        alternative: AlternativeId,
        fact: FactId,
    },
    UnreachableAlternativeOwner {
        alternative: AlternativeId,
        owner: BeatId,
    },
    UnreachableAlternativeEntry {
        alternative: AlternativeId,
        owner: BeatId,
        entry: BeatId,
    },
    AlternativeExitNotTerminal {
        alternative: AlternativeId,
        exit: BeatId,
    },
    UnreachableAlternativeExit {
        alternative: AlternativeId,
        entry: BeatId,
        exit: BeatId,
    },
}

struct WorkBudget {
    remaining_work: usize,
}

impl WorkBudget {
    fn charge(&mut self, units: usize) -> Result<(), GraphLimit> {
        self.remaining_work = self
            .remaining_work
            .checked_sub(units)
            .ok_or(GraphLimit::Work)?;
        Ok(())
    }
}

fn find_reachable_nodes(
    edges: &[Vec<usize>],
    start: usize,
    budget: &mut WorkBudget,
) -> Result<Vec<bool>, GraphLimit> {
    budget.charge(edges.len())?;
    let mut reachable = vec![false; edges.len()];
    let mut pending_indices = vec![start];
    // Indices are created internally from the validated, enumerated beat map.
    reachable[start] = true;
    while let Some(current) = pending_indices.pop() {
        budget.charge(1)?;
        for next in &edges[current] {
            budget.charge(1)?;
            if !reachable[*next] {
                reachable[*next] = true;
                pending_indices.push(*next);
            }
        }
    }
    Ok(reachable)
}

fn find_cycle(
    edges: &[Vec<usize>],
    budget: &mut WorkBudget,
) -> Result<Option<(usize, usize)>, GraphLimit> {
    budget.charge(edges.len())?;
    let mut visit_states = vec![0_u8; edges.len()];
    let mut pending_visits = Vec::new();
    for (starting_index, _) in edges.iter().enumerate() {
        budget.charge(1)?;
        if visit_states[starting_index] != 0 {
            continue;
        }
        visit_states[starting_index] = 1;
        pending_visits.push((starting_index, 0));
        while let Some((current, next_index)) = pending_visits.last_mut() {
            budget.charge(1)?;
            let Some(next) = edges[*current].get(*next_index).copied() else {
                visit_states[*current] = 2;
                pending_visits.pop();
                continue;
            };
            *next_index += 1;
            if visit_states[next] == 1 {
                return Ok(Some((*current, next)));
            }
            if visit_states[next] == 0 {
                visit_states[next] = 1;
                pending_visits.push((next, 0));
            }
        }
    }
    Ok(None)
}

/// Validates closure, all-component acyclicity, a reachable terminal, and each explicit route.
/// Success proves only structural reachability. Runtime predicates, rights, source/rules
/// compatibility, publication, and recipient authorization remain with their existing owners.
/// The graph is never mutated; refusal never produces an accepted pack or truncated success.
pub fn validate_graph<BeatId: Ord + Clone, FactId: Ord + Clone, AlternativeId: Ord + Clone>(
    graph: &GraphView<'_, BeatId, FactId, AlternativeId>,
    limits: GraphLimits,
) -> Result<(), GraphError<BeatId, FactId, AlternativeId>> {
    for (count, maximum, limit) in [
        (graph.beats.len(), limits.max_beats, GraphLimit::Beats),
        (graph.facts.len(), limits.max_facts, GraphLimit::Facts),
        (
            graph.alternatives.len(),
            limits.max_alternatives,
            GraphLimit::Alternatives,
        ),
    ] {
        if count > maximum {
            return Err(GraphError::LimitExceeded(limit));
        }
    }
    let mut budget = WorkBudget {
        remaining_work: limits.max_work,
    };
    let mut beat_indices = BTreeMap::new();
    let mut edge_count = 0_usize;
    for (index, beat) in graph.beats.iter().enumerate() {
        budget.charge(1).map_err(GraphError::LimitExceeded)?;
        edge_count = edge_count
            .checked_add(beat.next.len())
            .ok_or(GraphError::LimitExceeded(GraphLimit::Edges))?;
        if edge_count > limits.max_edges {
            return Err(GraphError::LimitExceeded(GraphLimit::Edges));
        }
        if beat_indices.insert(beat.id, index).is_some() {
            return Err(GraphError::DuplicateBeat(beat.id.clone()));
        }
    }
    let mut facts = BTreeSet::new();
    for fact in graph.facts {
        budget.charge(1).map_err(GraphError::LimitExceeded)?;
        if !facts.insert(fact) {
            return Err(GraphError::DuplicateFact(fact.clone()));
        }
    }
    let mut alternatives = BTreeSet::new();
    for alternative in graph.alternatives {
        budget.charge(1).map_err(GraphError::LimitExceeded)?;
        if !alternatives.insert(alternative.id) {
            return Err(GraphError::DuplicateAlternative(alternative.id.clone()));
        }
    }
    let start = beat_indices
        .get(graph.start)
        .copied()
        .ok_or_else(|| GraphError::MissingStart(graph.start.clone()))?;
    let mut edges = Vec::with_capacity(graph.beats.len());
    for beat in graph.beats {
        budget.charge(1).map_err(GraphError::LimitExceeded)?;
        let mut next_indices = Vec::with_capacity(beat.next.len());
        for next in beat.next {
            budget.charge(1).map_err(GraphError::LimitExceeded)?;
            let index = beat_indices
                .get(next)
                .copied()
                .ok_or_else(|| GraphError::MissingEdge {
                    from: beat.id.clone(),
                    to: next.clone(),
                })?;
            next_indices.push(index);
        }
        edges.push(next_indices);
    }
    if let Some((from, to)) = find_cycle(&edges, &mut budget).map_err(GraphError::LimitExceeded)? {
        return Err(GraphError::Cycle {
            from: graph.beats[from].id.clone(),
            to: graph.beats[to].id.clone(),
        });
    }
    let reachable_from_start =
        find_reachable_nodes(&edges, start, &mut budget).map_err(GraphError::LimitExceeded)?;
    let mut has_reachable_exit = false;
    for (index, beat) in graph.beats.iter().enumerate() {
        budget.charge(1).map_err(GraphError::LimitExceeded)?;
        has_reachable_exit |= beat.terminal && reachable_from_start[index];
    }
    if !has_reachable_exit {
        return Err(GraphError::NoReachableExit);
    }
    for alternative in graph.alternatives {
        budget.charge(1).map_err(GraphError::LimitExceeded)?;
        if let Some(fact) = alternative.disclosure_fact
            && !facts.contains(fact)
        {
            return Err(GraphError::MissingDisclosureFact {
                alternative: alternative.id.clone(),
                fact: fact.clone(),
            });
        }
        let owner = beat_indices
            .get(alternative.owner)
            .copied()
            .ok_or_else(|| GraphError::MissingAlternativeOwner {
                alternative: alternative.id.clone(),
                owner: alternative.owner.clone(),
            })?;
        let entry = beat_indices
            .get(alternative.entry)
            .copied()
            .ok_or_else(|| GraphError::MissingAlternativeEntry {
                alternative: alternative.id.clone(),
                entry: alternative.entry.clone(),
            })?;
        let exit = beat_indices.get(alternative.exit).copied().ok_or_else(|| {
            GraphError::MissingAlternativeExit {
                alternative: alternative.id.clone(),
                exit: alternative.exit.clone(),
            }
        })?;
        if !reachable_from_start[owner] {
            return Err(GraphError::UnreachableAlternativeOwner {
                alternative: alternative.id.clone(),
                owner: alternative.owner.clone(),
            });
        }
        if !find_reachable_nodes(&edges, owner, &mut budget).map_err(GraphError::LimitExceeded)?
            [entry]
        {
            return Err(GraphError::UnreachableAlternativeEntry {
                alternative: alternative.id.clone(),
                owner: alternative.owner.clone(),
                entry: alternative.entry.clone(),
            });
        }
        if !graph.beats[exit].terminal {
            return Err(GraphError::AlternativeExitNotTerminal {
                alternative: alternative.id.clone(),
                exit: alternative.exit.clone(),
            });
        }
        if !find_reachable_nodes(&edges, entry, &mut budget).map_err(GraphError::LimitExceeded)?
            [exit]
        {
            return Err(GraphError::UnreachableAlternativeExit {
                alternative: alternative.id.clone(),
                entry: alternative.entry.clone(),
                exit: alternative.exit.clone(),
            });
        }
    }
    Ok(())
}
