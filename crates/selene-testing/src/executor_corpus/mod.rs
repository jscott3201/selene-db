//! Curated executor snapshot corpus fixtures.
#![allow(missing_docs)]

pub mod coverage;
pub mod fixtures;

use selene_core::{IStr, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, IndexHandle, IndexKind, JoinTree, ProcedureHandle,
    ProcedureResult, ScanAccess, analyze, optimize, parse, plan,
};

use crate::{MockIndexCatalog, MockProcedureRegistry};

pub use coverage::{ExecutorOperator, PHASE_A_OPERATORS};
pub use fixtures::ExecutorCorpusFixture;

#[derive(Clone, Debug)]
pub struct ExecutorCorpus {
    entries: Vec<ExecutorCorpusEntry>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ExecutorCorpusEntry {
    pub slug: &'static str,
    pub program: ExecutorCorpusProgram,
    pub fixture: ExecutorCorpusFixture,
    pub category: ExecutorCorpusCategory,
    pub uses_index_catalog: bool,
    pub registry: ExecutorCorpusRegistry,
    pub covered_operators: &'static [ExecutorOperator],
}

#[derive(Clone, Copy, Debug)]
pub enum ExecutorCorpusProgram {
    Single(&'static str),
    Sequence(&'static [&'static str]),
    HandBuilt(fn() -> ExecutionPlan),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ExecutorCorpusCategory {
    Read,
    Mutation,
    Ddl,
    Catalog,
    Call,
    Transaction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutorCorpusRegistry {
    Empty,
    StandardMock,
}

impl ExecutorCorpus {
    #[must_use]
    pub fn m5d() -> Self {
        Self {
            entries: ENTRIES.to_vec(),
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &ExecutorCorpusEntry> {
        self.entries.iter()
    }

    #[must_use]
    pub fn standard_mock_catalog() -> MockIndexCatalog {
        let person = istr("Person");
        MockIndexCatalog::new()
            .with_node_label_index(person)
            .with_node_typed_index(person, istr("age"), IndexKind::Integer)
            .with_node_typed_index(person, istr("email"), IndexKind::String)
            .with_node_typed_index(person, istr("kind"), IndexKind::String)
            .with_node_typed_index(person, istr("tenant"), IndexKind::String)
            .with_node_composite_index(person, vec![istr("tenant"), istr("kind")])
    }

    #[must_use]
    pub fn standard_mock_registry() -> MockProcedureRegistry {
        crate::PlanCorpus::standard_mock_registry().with_result(
            ProcedureHandle::new(1),
            ProcedureResult {
                rows: vec![
                    vec![Value::String(istr("alpha")), Value::Int(10)],
                    vec![Value::String(istr("beta")), Value::Int(20)],
                ],
            },
        )
    }
}

use ExecutorCorpusCategory::*;
use ExecutorCorpusFixture::*;
use ExecutorCorpusProgram::*;
use ExecutorCorpusRegistry::*;

const SEEDED: ExecutorCorpusFixture = StandardSeeded {
    node_count: 4,
    with_indexes: false,
};
const SEEDED_INDEXED: ExecutorCorpusFixture = StandardSeeded {
    node_count: 4,
    with_indexes: true,
};

macro_rules! e {
    ($slug:literal, $program:expr, $fixture:expr, $category:expr, $indexed:expr, $registry:expr, [$($op:ident),+ $(,)?]) => {
        ExecutorCorpusEntry {
            slug: $slug,
            program: $program,
            fixture: $fixture,
            category: $category,
            uses_index_catalog: $indexed,
            registry: $registry,
            covered_operators: &[$(ExecutorOperator::$op),+],
        }
    };
}

const ENTRIES: &[ExecutorCorpusEntry] = &[
    e!(
        "read_return_literal",
        Single("RETURN 1 AS one"),
        EmptyOpen,
        Read,
        false,
        Empty,
        [Project]
    ),
    e!(
        "read_match_filter_project",
        Single("MATCH (n:Person) FILTER n.score > 1 RETURN n.name AS name"),
        SEEDED,
        Read,
        false,
        Empty,
        [LinearScan, Filter, Project]
    ),
    e!(
        "read_label_index_orderby",
        HandBuilt(handbuilt_label_index_orderby),
        SEEDED,
        Read,
        false,
        Empty,
        [LabelIndexScan, OrderBy, Project]
    ),
    e!(
        "read_typed_index_range_topk",
        Single("MATCH (n:Person) FILTER n.age >= 21 RETURN n.name AS name ORDER BY n.age LIMIT 2"),
        SEEDED_INDEXED,
        Read,
        true,
        Empty,
        [TypedIndexRangeScan, TopK, Project]
    ),
    e!(
        "read_bitmap_union_limit",
        Single(
            "MATCH (n:Person) FILTER n.email IN ['a@example.com', 'b@example.com'] RETURN n.name AS name LIMIT 3"
        ),
        SEEDED_INDEXED,
        Read,
        true,
        Empty,
        [BitmapUnionScan, Limit, Project]
    ),
    e!(
        "read_composite_lookup",
        Single(
            "MATCH (n:Person) FILTER n.tenant = 't1' AND n.kind = 'person' RETURN n.name AS name"
        ),
        SEEDED_INDEXED,
        Read,
        true,
        Empty,
        [CompositeLookupScan, Project]
    ),
    e!(
        "read_expand_hash_join",
        Single("MATCH (a:Person) MATCH (a)-[:KNOWS]->(b) RETURN a, b"),
        SEEDED,
        Read,
        false,
        Empty,
        [Expand, HashJoin, LinearScan, Project]
    ),
    e!(
        "read_optional_outer",
        Single("MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b) RETURN a, b"),
        SEEDED,
        Read,
        false,
        Empty,
        [Outer, Expand, Project]
    ),
    e!(
        "read_subplan",
        HandBuilt(handbuilt_subplan_read),
        SEEDED,
        Read,
        false,
        Empty,
        [Subplan, Project]
    ),
    e!(
        "read_let_unwind_distinct_limit",
        Single("UNWIND [1, 2, 2] AS x LET y = x RETURN DISTINCT y LIMIT 2"),
        EmptyOpen,
        Read,
        false,
        Empty,
        [Unwind, Let, Distinct, Limit, Project]
    ),
    e!(
        "read_group_by_aggregate",
        Single(
            "MATCH (n:Person) RETURN n.tenant AS tenant, count(*) AS c, sum(n.score) AS s GROUP BY n.tenant ORDER BY tenant"
        ),
        SEEDED,
        Read,
        false,
        Empty,
        [GroupBy, Aggregate, OrderBy]
    ),
    e!(
        "read_union_all",
        Single("RETURN 1 AS n UNION ALL RETURN 2 AS n"),
        EmptyOpen,
        Read,
        false,
        Empty,
        [Union, Project]
    ),
    e!(
        "read_chain",
        Single("RETURN 1 AS a NEXT RETURN 2 AS b"),
        EmptyOpen,
        Read,
        false,
        Empty,
        [Chain, Project]
    ),
    e!(
        "mutation_insert_node_return",
        Single("INSERT (n:Person {name: 'zed', score: 11}) RETURN n"),
        EmptyOpen,
        Mutation,
        false,
        Empty,
        [MutationInsert, Project]
    ),
    e!(
        "mutation_match_set_return",
        Single("MATCH (n:Person {name: 'a'}) SET n.score = 42 RETURN n.score AS score"),
        SEEDED,
        Mutation,
        false,
        Empty,
        [LinearScan, MutationSet, Project]
    ),
    e!(
        "mutation_remove_delete",
        Single("MATCH (n:Person {name: 'd'}) REMOVE n.score DELETE n FINISH"),
        SEEDED,
        Mutation,
        false,
        Empty,
        [LinearScan, MutationRemove, MutationDelete]
    ),
    e!(
        "ddl_create_node_type_then_insert",
        HandBuilt(handbuilt_create_node_type_then_insert),
        EmptyClosed,
        Ddl,
        false,
        Empty,
        [CatalogCreate, MutationInsert, Project]
    ),
    e!(
        "ddl_drop_node_type",
        Single("DROP NODE TYPE :Person"),
        EmptyPersonClosed,
        Ddl,
        false,
        Empty,
        [CatalogDrop]
    ),
    e!(
        "catalog_show_node_types",
        Single("SHOW NODE TYPES"),
        EmptyPersonClosed,
        Catalog,
        false,
        Empty,
        [CatalogShow]
    ),
    e!(
        "transaction_explicit_commit",
        Sequence(&[
            "START TRANSACTION",
            "INSERT (n:Person {name: 'tx'}) FINISH",
            "COMMIT"
        ]),
        EmptyOpen,
        Transaction,
        false,
        Empty,
        [TxStart, MutationInsert, TxCommit]
    ),
    e!(
        "transaction_explicit_rollback",
        Sequence(&[
            "START TRANSACTION",
            "INSERT (n:Person {name: 'rolled'}) FINISH",
            "ROLLBACK"
        ]),
        EmptyOpen,
        Transaction,
        false,
        Empty,
        [TxStart, MutationInsert, TxRollback]
    ),
    e!(
        "call_read_tier_yield_named",
        Single("CALL pkg.all() YIELD outA AS first, outB AS second"),
        EmptyOpen,
        Call,
        false,
        StandardMock,
        [Call]
    ),
];

fn handbuilt_label_index_orderby() -> ExecutionPlan {
    let mut plan = planned(
        "MATCH (n:Person) RETURN n ORDER BY n.name",
        &EmptyProcedureRegistry,
    );
    let scan = first_scan_mut(
        &mut plan
            .pattern_plan
            .as_mut()
            .expect("fixture has pattern")
            .join_tree,
    )
    .expect("fixture has scan");
    scan.access = ScanAccess::LabelIndex {
        handle: IndexHandle::new(1),
    };
    plan
}

fn handbuilt_subplan_read() -> ExecutionPlan {
    let mut plan = planned("MATCH (n:Person) RETURN n", &EmptyProcedureRegistry);
    let mut subplan = plan.clone();
    subplan.pipeline.clear();
    plan.pattern_plan
        .as_mut()
        .expect("fixture has pattern")
        .join_tree = JoinTree::Subplan(Box::new(subplan));
    plan
}

fn handbuilt_create_node_type_then_insert() -> ExecutionPlan {
    let mut plan = planned(
        "INSERT (n:Person {name: 'Ada'}) RETURN n",
        &EmptyProcedureRegistry,
    );
    let create = planned(
        "CREATE NODE TYPE :Person (name :: STRING)",
        &EmptyProcedureRegistry,
    )
    .pipeline
    .remove(0);
    plan.category = selene_gql::StatementCategory::CatalogModifying;
    plan.pipeline.insert(0, create);
    plan
}

fn planned(source: &str, registry: &dyn selene_gql::ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("executor corpus source parses");
    let analyzed = analyze(statement, registry, None).expect("executor corpus source analyzes");
    let plan = plan(&analyzed, registry).expect("executor corpus source plans");
    let ctx = selene_gql::OptimizeContext::default();
    optimize(plan, &ctx)
}

fn first_scan_mut(tree: &mut JoinTree) -> Option<&mut selene_gql::NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. } => first_scan_mut(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan_mut(left).or_else(|| first_scan_mut(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("executor fixture strings fit the interner")
}
