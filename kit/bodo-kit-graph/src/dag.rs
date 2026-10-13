//! An incrementally maintained topological order, which rejects edges that would close a cycle.
//!
//! [`IncrementalDag`] keeps a topological order of its nodes up to date as edges are added, with the algorithm of
//! Pearce and Kelly ("A dynamic topological sort algorithm for directed acyclic graphs", 2006). Adding an edge that
//! already agrees with the order costs O(1). Otherwise only the nodes between the two ends of the edge are searched
//! and reordered; the worst case is O(V + E). An edge that would close a cycle is refused, and the error names the
//! cycle, so the user or an agent can see which connections form the loop (project.md section 9.4).

use std::collections::{HashMap, HashSet};

/// A node of an [`IncrementalDag`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DagNode(pub u32);

/// Why an edge was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DagError {
    /// The edge would close a cycle. The nodes are listed in cycle order: the refused edge goes from the first to the
    /// second, each node has an edge to the next, and the last has an edge back to the first.
    #[error("the connection would create a cycle through {} nodes", .path.len())]
    Cycle {
        /// The cycle, starting at the edge's source.
        path: Vec<DagNode>,
    },
    /// One end of the edge is not a node of the graph.
    #[error("node {} is not in the graph", .0.0)]
    UnknownNode(DagNode),
}

/// A directed acyclic graph that keeps a topological order as it changes.
#[derive(Clone, Debug, Default)]
pub struct IncrementalDag {
    /// Position of each node in the topological order; `None` for free slots.
    order: Vec<Option<u32>>,
    successors: Vec<Vec<u32>>,
    predecessors: Vec<Vec<u32>>,
    free: Vec<u32>,
    next_position: u32,
}

impl IncrementalDag {
    /// An empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node and returns it. It comes last in the order.
    ///
    /// Complexity: O(1) amortized.
    pub fn add_node(&mut self) -> DagNode {
        let position = self.next_position;
        self.next_position += 1;
        if let Some(index) = self.free.pop() {
            let slot = index as usize;
            self.order[slot] = Some(position);
            DagNode(index)
        } else {
            self.order.push(Some(position));
            self.successors.push(Vec::new());
            self.predecessors.push(Vec::new());
            DagNode(self.order.len() as u32 - 1)
        }
    }

    /// Removes a node and every edge touching it. Returns whether it existed.
    ///
    /// Complexity: O(d) in the edges of the node's neighbors.
    pub fn remove_node(&mut self, node: DagNode) -> bool {
        if !self.contains(node) {
            return false;
        }
        let index = node.0 as usize;
        for successor in std::mem::take(&mut self.successors[index]) {
            self.predecessors[successor as usize].retain(|&p| p != node.0);
        }
        for predecessor in std::mem::take(&mut self.predecessors[index]) {
            self.successors[predecessor as usize].retain(|&s| s != node.0);
        }
        self.order[index] = None;
        self.free.push(node.0);
        true
    }

    /// Whether `node` is in the graph.
    pub fn contains(&self, node: DagNode) -> bool {
        matches!(self.order.get(node.0 as usize), Some(Some(_)))
    }

    /// Adds an edge from `from` to `to`, or refuses it when it would close a cycle. Parallel edges are allowed.
    ///
    /// Complexity: O(1) when the order already agrees; otherwise O(k log k) in the k nodes between the two ends,
    /// plus their edges. O(V + E) in the worst case.
    pub fn add_edge(&mut self, from: DagNode, to: DagNode) -> Result<(), DagError> {
        let from_position = self.position(from).ok_or(DagError::UnknownNode(from))?;
        let to_position = self.position(to).ok_or(DagError::UnknownNode(to))?;
        if from == to {
            return Err(DagError::Cycle { path: vec![from] });
        }
        if from_position > to_position {
            self.reorder(from, to, from_position, to_position)?;
        }
        self.successors[from.0 as usize].push(to.0);
        self.predecessors[to.0 as usize].push(from.0);
        Ok(())
    }

    /// Removes one edge from `from` to `to`. Returns whether there was one.
    ///
    /// Complexity: O(d) in the degree of the two nodes.
    pub fn remove_edge(&mut self, from: DagNode, to: DagNode) -> bool {
        let (Some(successors), Some(predecessors)) = (self.successors.get_mut(from.0 as usize), self.predecessors.get_mut(to.0 as usize)) else {
            return false;
        };
        let Some(i) = successors.iter().position(|&s| s == to.0) else { return false };
        successors.swap_remove(i);
        if let Some(j) = predecessors.iter().position(|&p| p == from.0) {
            predecessors.swap_remove(j);
        }
        true
    }

    /// Whether an edge from `from` to `to` could be added without closing a cycle.
    ///
    /// Complexity: O(1) when the order already agrees, otherwise O(V + E) in the worst case.
    pub fn can_add_edge(&self, from: DagNode, to: DagNode) -> bool {
        match (self.position(from), self.position(to)) {
            (Some(f), Some(t)) => from != to && (f < t || self.forward_search(to, from, f).is_ok()),
            _ => false,
        }
    }

    /// Every node, in topological order: each edge goes from an earlier node to a later one.
    ///
    /// Complexity: O(V log V).
    pub fn topological_order(&self) -> Vec<DagNode> {
        let mut nodes: Vec<(u32, u32)> = self.order.iter().enumerate().filter_map(|(index, position)| position.map(|p| (p, index as u32))).collect();
        nodes.sort_unstable();
        nodes.into_iter().map(|(_, index)| DagNode(index)).collect()
    }

    fn position(&self, node: DagNode) -> Option<u32> {
        self.order.get(node.0 as usize).copied().flatten()
    }

    /// Searches forward from `start` through nodes no later than `upper`. Fails with the path when it reaches
    /// `target`; otherwise returns the nodes it visited.
    fn forward_search(&self, start: DagNode, target: DagNode, upper: u32) -> Result<Vec<u32>, Vec<DagNode>> {
        let mut parent: HashMap<u32, u32> = HashMap::new();
        let mut visited = vec![start.0];
        let mut stack = vec![start.0];
        parent.insert(start.0, start.0);
        while let Some(node) = stack.pop() {
            for &next in &self.successors[node as usize] {
                if next == target.0 {
                    // Walk back from `node` to `start` to recover the path start -> ... -> node.
                    let mut path = vec![DagNode(node)];
                    let mut current = node;
                    while current != start.0 {
                        let Some(&previous) = parent.get(&current) else { break };
                        current = previous;
                        path.push(DagNode(current));
                    }
                    path.reverse();
                    return Err(path);
                }
                let in_range = self.position(DagNode(next)).is_some_and(|p| p <= upper);
                if in_range && !parent.contains_key(&next) {
                    parent.insert(next, node);
                    visited.push(next);
                    stack.push(next);
                }
            }
        }
        Ok(visited)
    }

    /// Restores the order after an edge `from -> to` that goes backwards in it, or reports the cycle it would close.
    fn reorder(&mut self, from: DagNode, to: DagNode, from_position: u32, to_position: u32) -> Result<(), DagError> {
        let forward = self.forward_search(to, from, from_position).map_err(|tail| {
            let mut path = vec![from];
            path.extend(tail);
            DagError::Cycle { path }
        })?;
        // Search backward from `from` through nodes no earlier than `to`.
        let mut backward = vec![from.0];
        let mut seen = HashSet::from([from.0]);
        let mut stack = vec![from.0];
        while let Some(node) = stack.pop() {
            for &previous in &self.predecessors[node as usize] {
                let in_range = self.position(DagNode(previous)).is_some_and(|p| p >= to_position);
                if in_range && seen.insert(previous) {
                    backward.push(previous);
                    stack.push(previous);
                }
            }
        }
        // Everything that reaches `from` must now come before everything `to` reaches. Reuse the same positions.
        let by_position = |nodes: &mut Vec<u32>, order: &[Option<u32>]| nodes.sort_unstable_by_key(|&n| order[n as usize]);
        let mut forward = forward;
        by_position(&mut backward, &self.order);
        by_position(&mut forward, &self.order);
        let mut positions: Vec<u32> = backward.iter().chain(&forward).filter_map(|&n| self.order[n as usize]).collect();
        positions.sort_unstable();
        for (node, position) in backward.into_iter().chain(forward).zip(positions) {
            self.order[node as usize] = Some(position);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dag(n: usize) -> (IncrementalDag, Vec<DagNode>) {
        let mut dag = IncrementalDag::new();
        let nodes = (0..n).map(|_| dag.add_node()).collect();
        (dag, nodes)
    }

    fn assert_order_respects_edges(dag: &IncrementalDag) {
        let order = dag.topological_order();
        let rank: HashMap<DagNode, usize> = order.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        for (from, successors) in dag.successors.iter().enumerate() {
            for &to in successors {
                assert!(rank[&DagNode(from as u32)] < rank[&DagNode(to)], "edge {from} -> {to} breaks the order");
            }
        }
    }

    #[test]
    fn backward_edges_reorder() {
        let (mut dag, n) = dag(4);
        dag.add_edge(n[3], n[0]).unwrap();
        dag.add_edge(n[2], n[3]).unwrap();
        dag.add_edge(n[1], n[2]).unwrap();
        assert_order_respects_edges(&dag);
        assert_eq!(dag.topological_order(), [n[1], n[2], n[3], n[0]]);
    }

    #[test]
    fn cycles_are_refused_with_their_path() {
        let (mut dag, n) = dag(3);
        dag.add_edge(n[0], n[1]).unwrap();
        dag.add_edge(n[1], n[2]).unwrap();
        assert_eq!(dag.add_edge(n[2], n[0]), Err(DagError::Cycle { path: vec![n[2], n[0], n[1]] }));
        assert_eq!(dag.add_edge(n[1], n[1]), Err(DagError::Cycle { path: vec![n[1]] }));
        assert!(!dag.can_add_edge(n[2], n[0]));
        assert!(dag.can_add_edge(n[0], n[2]));
        assert_order_respects_edges(&dag);
    }

    #[test]
    fn removing_breaks_cycles_and_frees_slots() {
        let (mut dag, n) = dag(3);
        dag.add_edge(n[0], n[1]).unwrap();
        dag.add_edge(n[1], n[2]).unwrap();
        assert!(dag.remove_edge(n[1], n[2]));
        dag.add_edge(n[2], n[0]).unwrap();
        assert!(dag.remove_node(n[1]));
        assert!(!dag.contains(n[1]));
        assert_eq!(dag.add_edge(n[0], n[1]), Err(DagError::UnknownNode(n[1])));
        let reused = dag.add_node();
        assert_eq!(reused, n[1]);
        dag.add_edge(n[0], reused).unwrap();
        assert_order_respects_edges(&dag);
    }
}
