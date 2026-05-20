//! Curated planner and optimizer corpus fixtures.

use selene_core::{IStr, intern};
use selene_gql::{
    GqlType, IndexKind, ProcedureMutability, ProcedureOutputColumn, ProcedureParameter,
};

use crate::{MockIndexCatalog, MockProcedureRegistry};

/// Curated GQL statement corpus used by the plan-snapshot harness.
#[derive(Clone, Debug)]
pub struct PlanCorpus {
    entries: Vec<PlanCorpusEntry>,
}

/// One deterministic plan-corpus entry.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PlanCorpusEntry {
    /// Stable snake-case slug used as the snapshot suffix.
    pub slug: &'static str,
    /// Human-readable entry purpose.
    pub description: &'static str,
    /// GQL source text.
    pub source: &'static str,
    /// Coarse statement category.
    pub category: PlanCorpusCategory,
    /// Optimizer rule names expected to fire for this source.
    pub expected_rules: &'static [&'static str],
    /// Whether this entry should use [`PlanCorpus::standard_mock_catalog`].
    pub uses_index_catalog: bool,
    /// Procedure registry fixture used for analysis and planning.
    pub registry: PlanCorpusRegistry,
}

/// Coarse statement category for plan-corpus filtering.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum PlanCorpusCategory {
    /// Read-only query or query-composition statement.
    Read,
    /// Mutation pipeline.
    Mutation,
    /// Catalog DDL or SHOW statement.
    Ddl,
    /// Top-level CALL statement.
    Call,
    /// Transaction-control statement.
    Transaction,
    /// Catalog-inspection statement.
    Catalog,
}

/// Procedure registry fixture selector for a corpus entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PlanCorpusRegistry {
    /// Use `EmptyProcedureRegistry`.
    Empty,
    /// Use [`PlanCorpus::standard_mock_registry`].
    StandardMock,
}

impl PlanCorpus {
    /// Return the canonical plan-snapshot corpus.
    #[must_use]
    pub fn m5c() -> Self {
        Self {
            entries: ENTRIES.to_vec(),
        }
    }

    /// Iterate over corpus entries in deterministic order.
    pub fn entries(&self) -> impl Iterator<Item = &PlanCorpusEntry> {
        self.entries.iter()
    }

    /// Standard deterministic index catalog used by index-aware corpus entries.
    #[must_use]
    pub fn standard_mock_catalog() -> MockIndexCatalog {
        MockIndexCatalog::new()
            .with_node_typed_index(istr("Person"), istr("age"), IndexKind::Integer)
            .with_node_typed_index(istr("Person"), istr("email"), IndexKind::String)
            .with_node_typed_index(istr("Person"), istr("name"), IndexKind::String)
            .with_node_composite_index(istr("Purchase"), vec![istr("tenant"), istr("kind")])
    }

    /// Standard deterministic procedure registry used by CALL corpus entries.
    #[must_use]
    pub fn standard_mock_registry() -> MockProcedureRegistry {
        MockProcedureRegistry::new()
            .with_procedure(
                vec![istr("pkg"), istr("all")],
                Vec::new(),
                vec![
                    ProcedureOutputColumn::new(istr("outA"), GqlType::String),
                    ProcedureOutputColumn::new(istr("outB"), GqlType::Integer),
                ],
            )
            .with_procedure_mutability(
                vec![istr("pkg"), istr("mutate")],
                vec![ProcedureParameter::new(
                    istr("name"),
                    GqlType::String,
                    false,
                )],
                vec![ProcedureOutputColumn::new(
                    istr("changed"),
                    GqlType::Boolean,
                )],
                ProcedureMutability::GraphWrite,
            )
    }
}

const ENTRIES: &[PlanCorpusEntry] = &[
    PlanCorpusEntry {
        slug: "constant_fold_and_split",
        description: "Literal folding followed by root conjunction splitting.",
        source: "MATCH (n) WHERE n.score = 1 + 2 AND TRUE AND TRUE RETURN n",
        category: PlanCorpusCategory::Read,
        expected_rules: &[
            "constant_folding",
            "and_splitting",
            "node_filter_extraction",
        ],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "pipeline_filter_to_node_scan",
        description: "Pipeline FILTER pushdown into a node scan predicate bucket.",
        source: "MATCH (n:Person) FILTER n.age > 30 RETURN n",
        category: PlanCorpusCategory::Read,
        expected_rules: &["filter_pushdown", "node_filter_extraction"],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "expand_filter_to_edge",
        description: "Single-edge-binding filter pushed to an Expand edge.",
        source: "MATCH (a)-[e]->(b) FILTER e.weight > 1 RETURN a, b",
        category: PlanCorpusCategory::Read,
        expected_rules: &["filter_pushdown", "expand_filter_pushdown"],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "range_index_order_topk",
        description: "Range index access, order hint, and TopK fusion.",
        source: "MATCH (n:Person) FILTER n.age > 30 RETURN n ORDER BY n.age LIMIT 10",
        category: PlanCorpusCategory::Read,
        expected_rules: &[
            "filter_pushdown",
            "node_filter_extraction",
            "range_index_scan",
            "index_order",
            "top_k",
        ],
        uses_index_catalog: true,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "composite_index_with_residual",
        description: "Composite lookup consumes a subset and leaves a residual predicate.",
        source: "MATCH (n:Purchase) FILTER n.tenant = 'X' AND n.kind = 'Y' AND n.status = 'Z' RETURN n",
        category: PlanCorpusCategory::Read,
        expected_rules: &[
            "and_splitting",
            "filter_pushdown",
            "node_filter_extraction",
            "composite_index_lookup",
        ],
        uses_index_catalog: true,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "in_list_bitmap_union",
        description: "Small literal IN list uses typed-index bitmap union.",
        source: "MATCH (p:Person) FILTER p.email IN ['alice@example.com', 'bob@example.com'] RETURN p",
        category: PlanCorpusCategory::Read,
        expected_rules: &[
            "filter_pushdown",
            "node_filter_extraction",
            "in_list_optimization",
        ],
        uses_index_catalog: true,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "predicate_reorder_selectivity",
        description: "Less-selective inequality moves after equality.",
        source: "MATCH (n:Person) FILTER n.age <> 0 AND n.email = 'alice@example.com' RETURN n",
        category: PlanCorpusCategory::Read,
        expected_rules: &[
            "and_splitting",
            "filter_pushdown",
            "node_filter_extraction",
            "predicate_reorder",
        ],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "cyclic_pattern_wco",
        description: "Simple cycle becomes a marker-only WCO candidate.",
        source: "MATCH (a)-[:K]->(b)-[:K]->(c)-[:K]->(a) RETURN a, b, c",
        category: PlanCorpusCategory::Read,
        expected_rules: &["wco_join", "symmetry_breaking"],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "read_let_unwind_distinct",
        description: "Read pipeline surface covering Unwind, Let, Project, and Distinct.",
        source: "UNWIND [1, 2] AS x LET y = x RETURN DISTINCT y",
        category: PlanCorpusCategory::Read,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "read_group_by",
        description: "Grouping and aggregate pipeline surface.",
        source: "MATCH (n) RETURN length(n.name) AS l, count(*) AS c GROUP BY length(n.name)",
        category: PlanCorpusCategory::Read,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "read_union",
        description: "Set-composition pipeline surface.",
        source: "RETURN 1 AS n UNION ALL RETURN 2 AS n",
        category: PlanCorpusCategory::Read,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "read_chain",
        description: "NEXT chain pipeline surface.",
        source: "RETURN 1 AS a NEXT RETURN 2 AS b",
        category: PlanCorpusCategory::Read,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "insert_node_property_init",
        description: "INSERT lowering surface with property initializers.",
        source: "INSERT (:Person {name: 'Alice', age: 30})",
        category: PlanCorpusCategory::Mutation,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "create_node_type",
        description: "CREATE NODE TYPE DDL surface.",
        source: "CREATE NODE TYPE :Person (name :: STRING, age :: INT DEFAULT 0)",
        category: PlanCorpusCategory::Ddl,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "show_node_types",
        description: "SHOW NODE TYPES catalog-inspection surface.",
        source: "SHOW NODE TYPES",
        category: PlanCorpusCategory::Catalog,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
    PlanCorpusEntry {
        slug: "top_level_call_yield",
        description: "Top-level CALL with YIELD star and alias.",
        source: "CALL pkg.all() YIELD *, outA AS first",
        category: PlanCorpusCategory::Call,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::StandardMock,
    },
    PlanCorpusEntry {
        slug: "pipeline_call_extends_scope",
        description: "In-pipeline CALL extends the visible binding table.",
        source: "MATCH (n) CALL pkg.all() YIELD outA RETURN *",
        category: PlanCorpusCategory::Call,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::StandardMock,
    },
    PlanCorpusEntry {
        slug: "start_transaction",
        description: "Transaction-control lowering surface.",
        source: "START TRANSACTION",
        category: PlanCorpusCategory::Transaction,
        expected_rules: &[],
        uses_index_catalog: false,
        registry: PlanCorpusRegistry::Empty,
    },
];

fn istr(value: &str) -> IStr {
    intern(value).expect("test fixture strings fit the interner")
}
