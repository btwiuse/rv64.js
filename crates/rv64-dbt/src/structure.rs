//! Structured-control reconstruction for a bounded basic-block CFG.
//!
//! WebAssembly has no arbitrary `goto`.  Reducible SCCs become nested loops,
//! forward edges become exits from nested blocks, and only genuinely
//! multi-entry SCCs retain a local dispatcher.  Small multi-entry loops may be
//! duplicated instead, trading bounded code size for dispatcher-free hot
//! backedges.  This is the Stackifier family of algorithms expressed over
//! dense region-member indices rather than architecture-specific addresses.

use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Node = usize;
pub(crate) const ENTRY_NODE: Node = usize::MAX;

type Set = BTreeSet<Node>;
type Graph = BTreeMap<Node, Set>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Structure {
    Basic(Node),
    Dispatcher(Vec<Node>),
    Loop(Vec<Structure>),
    Block(Vec<Structure>),
}

impl Structure {
    pub(crate) fn head(&self) -> Vec<Node> {
        match self {
            Self::Basic(node) => vec![*node],
            Self::Dispatcher(entries) => entries.clone(),
            Self::Loop(children) | Self::Block(children) => {
                children.first().map_or_else(Vec::new, Self::head)
            }
        }
    }

    fn branches(&self, graph: &Graph, out: &mut Set) {
        match self {
            Self::Basic(node) => {
                if let Some(successors) = graph.get(node) {
                    out.extend(successors);
                }
            }
            Self::Dispatcher(entries) => out.extend(entries),
            Self::Loop(children) | Self::Block(children) => {
                for child in children {
                    child.branches(graph, out);
                }
            }
        }
    }
}

/// Reconstruct a structured program from internal CFG edges.
///
/// `entries` are the member indices callable from outside the generated
/// function. `duplication_limit` bounds the total number of basic-block copies
/// used to eliminate one multi-entry SCC; larger SCCs receive a localized
/// dispatcher instead.
pub(crate) fn stackify(
    successors: &[Vec<Node>],
    entries: &[Node],
    duplication_limit: usize,
) -> Vec<Structure> {
    let mut graph = Graph::new();
    for (node, outgoing) in successors.iter().enumerate() {
        graph.insert(
            node,
            outgoing
                .iter()
                .copied()
                .filter(|target| *target < successors.len())
                .collect(),
        );
    }
    graph.insert(
        ENTRY_NODE,
        entries
            .iter()
            .copied()
            .filter(|entry| *entry < successors.len())
            .collect(),
    );

    let mut result = loopify(&graph, duplication_limit);
    blockify(&mut result, &graph);
    result
}

fn reverse(graph: &Graph) -> Graph {
    let mut reversed: Graph = graph.keys().map(|node| (*node, Set::new())).collect();
    for (&source, targets) in graph {
        for &target in targets {
            reversed.entry(target).or_default().insert(source);
        }
    }
    reversed
}

fn strongly_connected_components(graph: &Graph, reversed: &Graph) -> Vec<Vec<Node>> {
    fn visit(node: Node, graph: &Graph, seen: &mut Set, finished: &mut Vec<Node>) {
        if !seen.insert(node) {
            return;
        }
        if let Some(successors) = graph.get(&node) {
            for &successor in successors {
                visit(successor, graph, seen, finished);
            }
        }
        finished.push(node);
    }

    fn assign(node: Node, reversed: &Graph, assigned: &mut Set, group: &mut Vec<Node>) {
        if !assigned.insert(node) {
            return;
        }
        group.push(node);
        if let Some(predecessors) = reversed.get(&node) {
            for &predecessor in predecessors {
                assign(predecessor, reversed, assigned, group);
            }
        }
    }

    let mut finished = Vec::with_capacity(graph.len());
    let mut seen = Set::new();
    for &node in graph.keys() {
        visit(node, graph, &mut seen, &mut finished);
    }

    let mut assigned = Set::new();
    let mut groups = Vec::new();
    for &node in finished.iter().rev() {
        let mut group = Vec::new();
        assign(node, reversed, &mut assigned, &mut group);
        if !group.is_empty() {
            groups.push(group);
        }
    }
    groups
}

fn loopify(graph: &Graph, duplication_limit: usize) -> Vec<Structure> {
    let reversed = reverse(graph);
    let groups = strongly_connected_components(graph, &reversed);
    let mut result = Vec::new();

    for group in groups {
        if group.len() == 1 {
            let node = group[0];
            if node == ENTRY_NODE {
                result.push(Structure::Dispatcher(
                    graph
                        .get(&ENTRY_NODE)
                        .into_iter()
                        .flatten()
                        .copied()
                        .collect(),
                ));
            } else {
                let basic = Structure::Basic(node);
                if graph
                    .get(&node)
                    .is_some_and(|successors| successors.contains(&node))
                {
                    result.push(Structure::Loop(vec![basic]));
                } else {
                    result.push(basic);
                }
            }
            continue;
        }

        let members: Set = group.iter().copied().collect();
        let entries: Vec<Node> = group
            .iter()
            .copied()
            .filter(|node| {
                reversed.get(node).is_some_and(|predecessors| {
                    predecessors
                        .iter()
                        .any(|incoming| !members.contains(incoming))
                })
            })
            .collect();
        debug_assert!(!entries.is_empty(), "every reachable SCC has an entry");

        if entries.len().saturating_mul(group.len()) > duplication_limit {
            // Break every edge into an SCC header. The new dispatcher is its
            // sole loop header and redirects only those formerly irreducible
            // edges; all other edges remain structured.
            let entry_set: Set = entries.iter().copied().collect();
            let mut subgroup = Graph::new();
            for &node in &group {
                subgroup.insert(
                    node,
                    graph[&node]
                        .iter()
                        .copied()
                        .filter(|target| members.contains(target) && !entry_set.contains(target))
                        .collect(),
                );
            }
            let mut children = loopify(&subgroup, duplication_limit);
            if entries.len() > 1 {
                children.insert(0, Structure::Dispatcher(entries));
            }
            result.push(Structure::Loop(children));
        } else {
            // Give each entry a single-entry copy of the SCC by cutting its
            // incoming backedges. This removes runtime dispatch at a bounded
            // and explicitly controlled code-size cost.
            for entry in entries {
                let mut subgroup = Graph::new();
                for &node in &group {
                    subgroup.insert(
                        node,
                        graph[&node]
                            .iter()
                            .copied()
                            .filter(|target| members.contains(target) && *target != entry)
                            .collect(),
                    );
                }
                result.push(Structure::Loop(loopify(&subgroup, duplication_limit)));
            }
        }
    }
    result
}

fn blockify(structures: &mut Vec<Structure>, graph: &Graph) {
    let mut index = 0;
    while index < structures.len() {
        match &mut structures[index] {
            Structure::Loop(children) | Structure::Block(children) => blockify(children, graph),
            Structure::Basic(_) | Structure::Dispatcher(_) => {}
        }

        let heads: Set = structures[index].head().into_iter().collect();
        let source = (0..index).find(|&candidate| {
            let mut branches = Set::new();
            structures[candidate].branches(graph, &mut branches);
            !branches.is_disjoint(&heads)
        });
        let Some(source) = source else {
            index += 1;
            continue;
        };

        // A single immediately preceding basic block reaches its successor by
        // ordinary Wasm fallthrough and needs no scope.
        if source + 1 == index && matches!(structures[source], Structure::Basic(_)) {
            index += 1;
            continue;
        }

        let children: Vec<_> = structures.drain(source..index).collect();
        structures.insert(source, Structure::Block(children));
        // Skip the newly inserted scope and its forward-edge target.
        index = source + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_basics(structures: &[Structure], out: &mut Vec<Node>) {
        for structure in structures {
            match structure {
                Structure::Basic(node) => out.push(*node),
                Structure::Dispatcher(_) => {}
                Structure::Loop(children) | Structure::Block(children) => {
                    collect_basics(children, out)
                }
            }
        }
    }

    fn contains_loop(structures: &[Structure]) -> bool {
        structures.iter().any(|structure| match structure {
            Structure::Loop(_) => true,
            Structure::Block(children) => contains_loop(children),
            Structure::Basic(_) | Structure::Dispatcher(_) => false,
        })
    }

    fn dispatcher_count(structures: &[Structure]) -> usize {
        structures
            .iter()
            .map(|structure| match structure {
                Structure::Dispatcher(_) => 1,
                Structure::Loop(children) | Structure::Block(children) => {
                    dispatcher_count(children)
                }
                Structure::Basic(_) => 0,
            })
            .sum()
    }

    #[test]
    fn structures_a_diamond_without_duplicating_blocks() {
        // 0 -> {1,2}; 1 -> 3; 2 -> 3.
        let structures = stackify(&[vec![1, 2], vec![3], vec![3], vec![]], &[0], 32);
        let mut basics = Vec::new();
        collect_basics(&structures, &mut basics);
        basics.sort_unstable();
        assert_eq!(basics, [0, 1, 2, 3]);
        assert!(!contains_loop(&structures));
        assert_eq!(dispatcher_count(&structures), 1);
        assert!(structures
            .iter()
            .any(|node| matches!(node, Structure::Block(_))));
    }

    #[test]
    fn turns_a_single_entry_cycle_into_a_loop() {
        let structures = stackify(&[vec![1], vec![0, 2], vec![]], &[0], 32);
        assert!(contains_loop(&structures));
        assert_eq!(dispatcher_count(&structures), 1);
    }

    #[test]
    fn localizes_a_dispatcher_for_an_irreducible_cycle() {
        // Both 0 and 1 are callable entries and form one SCC. A zero
        // duplication budget forces the bounded dispatcher fallback.
        let structures = stackify(&[vec![1], vec![0, 2], vec![]], &[0, 1], 0);
        assert!(contains_loop(&structures));
        assert!(dispatcher_count(&structures) > 1);
    }

    #[test]
    fn duplicates_a_small_multi_entry_cycle_when_budget_allows() {
        let structures = stackify(&[vec![1], vec![0, 2], vec![]], &[0, 1], 8);
        let mut basics = Vec::new();
        collect_basics(&structures, &mut basics);
        assert!(basics.iter().filter(|&&node| node == 0).count() >= 2);
        assert_eq!(dispatcher_count(&structures), 1);
    }
}
