//! Deterministic feature-dependency closure over opaque identifiers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Whether a reachable dependency is connected by one or several edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DependencyRelation {
    Direct,
    Transitive,
}

/// A validated acyclic dependency graph and its transitive closure.
#[derive(Clone, Debug)]
pub(crate) struct ClosureGraph {
    direct: BTreeMap<String, BTreeSet<String>>,
    reachable: BTreeMap<String, BTreeSet<String>>,
}

impl ClosureGraph {
    pub(crate) fn build<'a>(
        nodes: impl IntoIterator<Item = &'a str>,
        edges: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let mut direct = BTreeMap::<String, BTreeSet<String>>::new();
        for node in nodes {
            direct.entry(node.to_owned()).or_default();
        }
        for (source, target) in edges {
            if !direct.contains_key(source) {
                return Err(format!("unknown implication source {source}"));
            }
            if !direct.contains_key(target) {
                return Err(format!("unknown implication target {target}"));
            }
            if !direct
                .get_mut(source)
                .expect("source existence checked")
                .insert(target.to_owned())
            {
                return Err(format!("duplicate implication edge {source} -> {target}"));
            }
        }

        reject_cycles(&direct)?;
        let reachable = direct
            .keys()
            .map(|node| (node.clone(), reachable_from(node, &direct)))
            .collect();
        Ok(Self { direct, reachable })
    }

    pub(crate) fn dependencies(&self, source: &str) -> impl Iterator<Item = &str> {
        self.reachable
            .get(source)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    pub(crate) fn relation(&self, source: &str, dependency: &str) -> Option<DependencyRelation> {
        if self.direct.get(source)?.contains(dependency) {
            Some(DependencyRelation::Direct)
        } else if self.reachable.get(source)?.contains(dependency) {
            Some(DependencyRelation::Transitive)
        } else {
            None
        }
    }

    pub(crate) fn shortest_path(&self, source: &str, target: &str) -> Option<Vec<String>> {
        if !self.direct.contains_key(source) || !self.direct.contains_key(target) {
            return None;
        }
        if source == target {
            return Some(vec![source.to_owned()]);
        }

        let mut queue = VecDeque::from([vec![source.to_owned()]]);
        let mut best_depth = BTreeMap::from([(source.to_owned(), 0_usize)]);
        while let Some(path) = queue.pop_front() {
            let node = path.last().expect("paths are nonempty");
            let depth = path.len();
            for dependency in &self.direct[node] {
                let mut candidate = path.clone();
                candidate.push(dependency.clone());
                if dependency == target {
                    return Some(candidate);
                }
                if best_depth
                    .get(dependency)
                    .is_none_or(|previous| depth <= *previous)
                {
                    best_depth.insert(dependency.clone(), depth);
                    queue.push_back(candidate);
                }
            }
        }
        None
    }

    pub(crate) fn closure_for<'a>(
        &self,
        roots: impl IntoIterator<Item = &'a str>,
    ) -> BTreeSet<String> {
        let mut closure = BTreeSet::new();
        for root in roots {
            closure.insert(root.to_owned());
            closure.extend(self.dependencies(root).map(str::to_owned));
        }
        closure
    }

    pub(crate) fn shortest_path_from<'a>(
        &self,
        roots: impl IntoIterator<Item = &'a str>,
        target: &str,
    ) -> Option<Vec<String>> {
        roots
            .into_iter()
            .filter_map(|root| self.shortest_path(root, target))
            .min_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)))
    }

    pub(crate) fn dependency_first_order<'a>(
        &self,
        roots: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        fn visit(
            node: &str,
            graph: &BTreeMap<String, BTreeSet<String>>,
            visited: &mut BTreeSet<String>,
            output: &mut Vec<String>,
        ) {
            if !visited.insert(node.to_owned()) {
                return;
            }
            for dependency in &graph[node] {
                visit(dependency, graph, visited, output);
            }
            output.push(node.to_owned());
        }

        let roots = roots.into_iter().collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::new();
        let mut output = Vec::new();
        for root in &roots {
            visit(root, &self.direct, &mut visited, &mut output);
        }
        output
    }
}

fn reachable_from(source: &str, graph: &BTreeMap<String, BTreeSet<String>>) -> BTreeSet<String> {
    let mut reachable = BTreeSet::new();
    let mut pending = graph[source].iter().cloned().collect::<Vec<_>>();
    while let Some(node) = pending.pop() {
        if reachable.insert(node.clone()) {
            pending.extend(graph[&node].iter().cloned());
        }
    }
    reachable
}

fn reject_cycles(graph: &BTreeMap<String, BTreeSet<String>>) -> Result<(), String> {
    fn visit<'a>(
        node: &'a str,
        graph: &'a BTreeMap<String, BTreeSet<String>>,
        state: &mut BTreeMap<&'a str, u8>,
        stack: &mut Vec<&'a str>,
    ) -> Result<(), String> {
        state.insert(node, 1);
        stack.push(node);
        for target in &graph[node] {
            if state[target.as_str()] == 1 {
                let start = stack.iter().position(|item| *item == target).unwrap_or(0);
                let mut cycle = stack[start..].to_vec();
                cycle.push(target);
                return Err(format!("implication cycle: {}", cycle.join(" -> ")));
            }
            if state[target.as_str()] == 0 {
                visit(target, graph, state, stack)?;
            }
        }
        stack.pop();
        state.insert(node, 2);
        Ok(())
    }

    let mut state = graph
        .keys()
        .map(|node| (node.as_str(), 0))
        .collect::<BTreeMap<_, _>>();
    let mut stack = Vec::new();
    for node in graph.keys().map(String::as_str) {
        if state[node] == 0 {
            visit(node, graph, &mut state, &mut stack)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> ClosureGraph {
        ClosureGraph::build(nodes.iter().copied(), edges.iter().copied()).unwrap()
    }

    #[test]
    fn chains_diamonds_and_disconnected_nodes_are_deterministic() {
        let graph = graph(
            &["A", "B", "C", "D", "Z"],
            &[("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")],
        );
        assert_eq!(graph.dependencies("A").collect::<Vec<_>>(), ["B", "C", "D"]);
        assert_eq!(graph.relation("A", "B"), Some(DependencyRelation::Direct));
        assert_eq!(
            graph.relation("A", "D"),
            Some(DependencyRelation::Transitive)
        );
        assert_eq!(graph.relation("A", "Z"), None);
        assert_eq!(graph.shortest_path("A", "D").unwrap(), ["A", "B", "D"]);
        assert_eq!(graph.dependency_first_order(["A"]), ["D", "B", "C", "A"]);
    }

    #[test]
    fn shortest_paths_use_lexical_tie_breaking() {
        let graph = graph(
            &["A", "B", "C", "D", "E", "F"],
            &[
                ("A", "C"),
                ("A", "B"),
                ("B", "E"),
                ("C", "D"),
                ("D", "F"),
                ("E", "F"),
            ],
        );
        assert_eq!(graph.shortest_path("A", "F").unwrap(), ["A", "B", "E", "F"]);
    }

    #[test]
    fn invalid_graphs_are_actionable_and_emit_no_order() {
        assert_eq!(
            ClosureGraph::build(["A"], [("A", "B")]).unwrap_err(),
            "unknown implication target B"
        );
        assert_eq!(
            ClosureGraph::build(["A", "B"], [("A", "B"), ("A", "B")]).unwrap_err(),
            "duplicate implication edge A -> B"
        );
        assert_eq!(
            ClosureGraph::build(["A", "B"], [("A", "B"), ("B", "A")]).unwrap_err(),
            "implication cycle: A -> B -> A"
        );
    }

    #[test]
    fn exhaustive_small_dags_match_reference_reachability() {
        for size in 1..=5 {
            let nodes = (0..size).map(|index| index.to_string()).collect::<Vec<_>>();
            let candidates = (0..size)
                .flat_map(|source| ((source + 1)..size).map(move |target| (source, target)))
                .collect::<Vec<_>>();
            for mask in 0..(1_usize << candidates.len()) {
                let edges = candidates
                    .iter()
                    .enumerate()
                    .filter(|(bit, _)| mask & (1 << bit) != 0)
                    .map(|(_, (source, target))| (nodes[*source].as_str(), nodes[*target].as_str()))
                    .collect::<Vec<_>>();
                let graph = ClosureGraph::build(nodes.iter().map(String::as_str), edges.clone())
                    .expect("forward edges form a DAG");
                for source in 0..size {
                    let mut reference = BTreeSet::new();
                    for target in (source + 1)..size {
                        if reference_path(source, target, &edges, &nodes) {
                            reference.insert(nodes[target].as_str());
                        }
                    }
                    assert_eq!(
                        graph.dependencies(&nodes[source]).collect::<BTreeSet<_>>(),
                        reference
                    );
                }
            }
        }
    }

    fn reference_path(
        source: usize,
        target: usize,
        edges: &[(&str, &str)],
        nodes: &[String],
    ) -> bool {
        let mut pending = vec![nodes[source].as_str()];
        let mut seen = BTreeSet::new();
        while let Some(node) = pending.pop() {
            for (_, next) in edges.iter().filter(|(from, _)| *from == node) {
                if *next == nodes[target] {
                    return true;
                }
                if seen.insert(*next) {
                    pending.push(next);
                }
            }
        }
        false
    }
}
