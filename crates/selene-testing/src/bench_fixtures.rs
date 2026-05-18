//! Deterministic graph fixtures for benchmark binaries.

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::{SeleneGraph, SharedGraph, TypedIndexKind};

/// Canonical publish-quality graph-size scales for release-prep benchmarks.
pub const BENCHMARK_SCALES: &[usize] = &[10_000, 50_000, 100_000];

/// Deterministic in-memory graph fixture used by benchmark binaries.
///
/// The graph contains `scale` nodes, roughly `3 * scale` directed edges, a
/// small fixed label vocabulary, and built-in typed indexes for hot property
/// lookups. The generator avoids filesystem state and caches so repeated
/// calls with the same scale produce the same graph shape.
#[derive(Clone, Debug)]
pub struct BenchFixture {
    scale: usize,
    graph: SeleneGraph,
    sample_node_id: NodeId,
    person_label: IStr,
    sensor_label: IStr,
    edge_label: IStr,
    age_key: IStr,
    bench_id_key: IStr,
    name_key: IStr,
    score_key: IStr,
    sample_age: i64,
    sample_name: IStr,
}

impl BenchFixture {
    /// Build the canonical benchmark graph at `scale` nodes.
    #[must_use]
    pub fn build(scale: usize) -> Self {
        let scale = scale.max(1);
        let person_label = istr("Person");
        let sensor_label = istr("Sensor");
        let device_label = istr("Device");
        let edge_label = istr("KNOWS");
        let age_key = istr("age");
        let bench_id_key = istr("bench_id");
        let name_key = istr("name");
        let score_key = istr("score");
        let shared = SharedGraph::new(GraphId::new(1));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let label = match idx % 3 {
                    0 => person_label,
                    1 => sensor_label,
                    _ => device_label,
                };
                let props = PropertyMap::from_pairs([
                    (age_key, Value::Int(sample_age(idx))),
                    (bench_id_key, Value::Int(idx as i64)),
                    (name_key, Value::String(sample_name(idx))),
                    (score_key, Value::Int((idx % 1_024) as i64)),
                ])
                .expect("fixture properties fit core caps");
                mutator
                    .create_node(LabelSet::single(label), props)
                    .expect("fixture node insert succeeds");
            }
            for idx in 0..scale {
                let source = NodeId::new(idx as u64 + 1);
                for offset in [1_usize, 7, 31] {
                    let target = NodeId::new(((idx + offset) % scale) as u64 + 1);
                    mutator
                        .create_edge(edge_label, source, target, PropertyMap::new())
                        .expect("fixture edge insert succeeds");
                }
            }
            txn.commit().expect("fixture commit succeeds");
        }
        shared
            .create_property_index(person_label, age_key, TypedIndexKind::I64)
            .expect("age index builds");
        shared
            .create_property_index(person_label, name_key, TypedIndexKind::String)
            .expect("name index builds");
        shared
            .create_property_index(person_label, bench_id_key, TypedIndexKind::I64)
            .expect("bench_id index builds");
        let graph = shared.read().as_ref().clone();
        Self {
            scale,
            graph,
            sample_node_id: NodeId::new(1),
            person_label,
            sensor_label,
            edge_label,
            age_key,
            bench_id_key,
            name_key,
            score_key,
            sample_age: sample_age(0),
            sample_name: sample_name(0),
        }
    }

    /// Borrow the immutable graph snapshot.
    #[must_use]
    pub const fn graph(&self) -> &SeleneGraph {
        &self.graph
    }

    /// Return the requested node scale.
    #[must_use]
    pub const fn scale(&self) -> usize {
        self.scale
    }

    /// Return a node ID known to exist.
    #[must_use]
    pub const fn sample_node_id(&self) -> NodeId {
        self.sample_node_id
    }

    /// Return the primary node label used by index benchmarks.
    #[must_use]
    pub const fn person_label(&self) -> IStr {
        self.person_label
    }

    /// Return the secondary node label used by label-scan benchmarks.
    #[must_use]
    pub const fn sensor_label(&self) -> IStr {
        self.sensor_label
    }

    /// Return the canonical edge label.
    #[must_use]
    pub const fn edge_label(&self) -> IStr {
        self.edge_label
    }

    /// Return the indexed integer property key.
    #[must_use]
    pub const fn age_key(&self) -> IStr {
        self.age_key
    }

    /// Return the unique integer lookup key used by write benchmarks.
    #[must_use]
    pub const fn bench_id_key(&self) -> IStr {
        self.bench_id_key
    }

    /// Return the indexed string property key.
    #[must_use]
    pub const fn name_key(&self) -> IStr {
        self.name_key
    }

    /// Return a non-indexed integer property key.
    #[must_use]
    pub const fn score_key(&self) -> IStr {
        self.score_key
    }

    /// Return an age value known to be present for [`Self::person_label`].
    #[must_use]
    pub const fn sample_age_value(&self) -> i64 {
        self.sample_age
    }

    /// Return a name value known to be present for [`Self::person_label`].
    #[must_use]
    pub const fn sample_name_value(&self) -> IStr {
        self.sample_name
    }

    /// Return one registered `(label, property)` pair for index lookup.
    #[must_use]
    pub const fn sample_label_property(&self) -> (IStr, IStr) {
        (self.person_label, self.age_key)
    }
}

/// Canonical write-side GQL corpus used by benchmark binaries.
pub struct WriteCorpus;

impl WriteCorpus {
    /// Single-node insert with scalar properties.
    #[must_use]
    pub const fn insert_single_node() -> &'static str {
        "INSERT (:Person { name: 'x', score: 42 })"
    }

    /// Indexed lookup plus node/edge insert.
    #[must_use]
    pub const fn insert_node_with_edge() -> &'static str {
        "MATCH (a:Person) FILTER a.bench_id = 42 INSERT (a)-[:KNOWS]->(:Person { name: 'x' })"
    }

    /// Indexed lookup plus scalar property update.
    #[must_use]
    pub const fn match_set() -> &'static str {
        "MATCH (n:Person) FILTER n.bench_id = 42 SET n.score = 17"
    }

    /// Indexed lookup plus detach-delete.
    #[must_use]
    pub const fn match_delete() -> &'static str {
        "MATCH (n:Person) FILTER n.bench_id = 42 DETACH DELETE n"
    }

    /// Explicit transaction statement sequence for multi-statement benches.
    #[must_use]
    pub const fn multi_statement_txn() -> &'static [&'static str] {
        &[
            "START TRANSACTION",
            "INSERT (:Person { name: 'a' })",
            "INSERT (:Person { name: 'b' })",
            "INSERT (:Person { name: 'c' })",
            "COMMIT",
        ]
    }
}

fn sample_age(idx: usize) -> i64 {
    20 + (idx % 80) as i64
}

fn sample_name(idx: usize) -> IStr {
    intern(&format!("bench-name-{}", idx % 256)).expect("fixture string fits interner")
}

fn istr(value: &str) -> IStr {
    intern(value).expect("fixture string fits interner")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use selene_gql::{EmptyProcedureRegistry, StatementCategory, analyze, parse};

    #[test]
    fn benchmark_scales_constant_is_pinned() {
        assert_eq!(BENCHMARK_SCALES, &[10_000, 50_000, 100_000]);
    }

    #[test]
    fn fixture_is_deterministic() {
        let a = BenchFixture::build(1_000);
        let b = BenchFixture::build(1_000);
        assert_eq!(a.graph().node_count(), b.graph().node_count());
        assert_eq!(a.graph().edge_count(), b.graph().edge_count());
        assert_eq!(a.graph().label_count(), b.graph().label_count());
        assert_eq!(
            a.graph().property_index_count(),
            b.graph().property_index_count()
        );
        assert_eq!(
            a.graph()
                .nodes_with_property_eq(
                    &a.person_label(),
                    &a.age_key(),
                    &Value::Int(a.sample_age_value())
                )
                .map(|rows| rows.len()),
            b.graph()
                .nodes_with_property_eq(
                    &b.person_label(),
                    &b.age_key(),
                    &Value::Int(b.sample_age_value())
                )
                .map(|rows| rows.len())
        );
    }

    #[test]
    fn fixture_scales_correctly() {
        for scale in [100, 1_000, 5_000] {
            let fixture = BenchFixture::build(scale);
            assert_eq!(fixture.graph().node_count(), scale);
            assert_eq!(fixture.graph().edge_count(), scale * 3);
        }
    }

    #[test]
    fn bench_fixture_assigns_unique_bench_id() {
        let fixture = BenchFixture::build(100);
        let mut ids = BTreeSet::new();
        for raw in 1..=100 {
            let node = NodeId::new(raw);
            let value = fixture
                .graph()
                .node_properties(node)
                .and_then(|properties| properties.get(&fixture.bench_id_key()))
                .expect("bench_id property exists");
            let Value::Int(bench_id) = value else {
                panic!("bench_id should be an integer, got {value:?}");
            };
            assert!(ids.insert(*bench_id));
        }
        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn write_corpus_statements_parse() {
        for source in write_corpus_sources() {
            parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error}"));
        }
    }

    #[test]
    fn write_corpus_categories_are_writes() {
        for source in write_corpus_sources() {
            let statement =
                parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error}"));
            let analyzed = analyze(statement, &EmptyProcedureRegistry, None)
                .unwrap_or_else(|error| panic!("{source} should analyze: {error}"));
            assert!(
                matches!(
                    analyzed.category,
                    StatementCategory::DataModifying | StatementCategory::TransactionControl
                ),
                "{source} should be data-modifying or transaction-control"
            );
        }
    }

    fn write_corpus_sources() -> Vec<&'static str> {
        let mut sources = vec![
            WriteCorpus::insert_single_node(),
            WriteCorpus::insert_node_with_edge(),
            WriteCorpus::match_set(),
            WriteCorpus::match_delete(),
        ];
        sources.extend_from_slice(WriteCorpus::multi_statement_txn());
        sources
    }
}
