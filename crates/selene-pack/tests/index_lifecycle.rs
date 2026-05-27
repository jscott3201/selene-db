//! `selene.create_index` / `selene.drop_index` integration tests.

use std::collections::HashMap;

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    CompositeIndexHandle, EmptyProcedureRegistry, ExecutionPlan, GqlType, IndexCatalog,
    IndexHandle, IndexKind, IndexTarget, JoinTree, NodeOrEdgeScan, OptimizeContext,
    ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata, ProcedureMutability,
    ProcedureOutputSchema, ProcedureParameter, ProcedureRegistry, ProcedureResult,
    ProcedureSignature, ProcedureTier, ScanAccess, StatementOutput, TypedIndexLookup, analyze,
    execute_statement, optimize, parse, plan,
};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_pack::ProcedurePackRegistry;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn props(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

fn execute_ok(
    source: &str,
    session: &mut selene_gql::Session<'_>,
    registry: &dyn ProcedureRegistry,
) {
    let plan = planned(source, registry);
    let output = execute_statement(&plan, session, registry).expect("statement executes");
    match output {
        StatementOutput::Empty => {}
        StatementOutput::Written(outcome) => assert!(
            outcome.rows.is_none(),
            "execute_ok received Written with rows; CALL site under test should not have RETURN",
        ),
        StatementOutput::Rows(_) => {
            panic!("execute_ok received Rows; CALL site under test should not produce rows")
        }
        other => panic!("execute_ok received unexpected StatementOutput variant: {other:?}"),
    }
}

fn execute_err(
    source: &str,
    session: &mut selene_gql::Session<'_>,
    planning: &dyn ProcedureRegistry,
    runtime: &dyn ProcedureRegistry,
) -> selene_gql::ExecutorError {
    let plan = planned(source, planning);
    execute_statement(&plan, session, runtime).expect_err("statement should fail")
}

#[test]
fn create_index_call_registers_property_index() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42101));
    let mut session = selene_gql::Session::new(&graph);

    execute_ok(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
    );

    assert!(
        graph
            .read()
            .property_index_for(&istr("Person"), &istr("age"))
            .is_some()
    );
}

#[test]
fn drop_index_call_removes_property_index() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42102));
    graph
        .create_property_index(istr("Person"), istr("age"), TypedIndexKind::I64)
        .unwrap();
    let mut session = selene_gql::Session::new(&graph);

    execute_ok(
        "CALL selene.drop_index('Person', 'age')",
        &mut session,
        &registry,
    );

    assert!(
        graph
            .read()
            .property_index_for(&istr("Person"), &istr("age"))
            .is_none()
    );
}

#[test]
fn drop_index_call_is_idempotent_when_absent() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42103));
    let mut session = selene_gql::Session::new(&graph);

    execute_ok(
        "CALL selene.drop_index('Person', 'age')",
        &mut session,
        &registry,
    );

    assert_eq!(graph.read().property_index_count(), 0);
}

#[test]
fn duplicate_create_index_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42104));
    let mut session = selene_gql::Session::new(&graph);
    execute_ok(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
    );

    let err = execute_err(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
        &registry,
    );

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn create_index_against_incompatible_existing_data_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42105));
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(istr("Person")),
                props([(istr("age"), Value::String(istr("old")))]),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
        &registry,
    );

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn create_drop_recreate_in_explicit_tx_commits_last_state() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42106));
    let mut session = selene_gql::Session::new(&graph);

    execute_ok("START TRANSACTION", &mut session, &registry);
    execute_ok(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
    );
    execute_ok(
        "CALL selene.drop_index('Person', 'age')",
        &mut session,
        &registry,
    );
    execute_ok(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
    );
    execute_ok("COMMIT", &mut session, &registry);

    assert!(
        graph
            .read()
            .property_index_for(&istr("Person"), &istr("age"))
            .is_some()
    );
}

#[test]
fn index_creation_in_explicit_tx_is_reverted_by_rollback() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42107));
    let mut session = selene_gql::Session::new(&graph);

    execute_ok("START TRANSACTION", &mut session, &registry);
    execute_ok(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
    );
    execute_ok("ROLLBACK", &mut session, &registry);

    assert!(
        graph
            .read()
            .property_index_for(&istr("Person"), &istr("age"))
            .is_none()
    );
}

#[test]
fn created_index_with_i64_kind_is_picked_by_optimizer_on_next_plan() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42108));
    let mut session = selene_gql::Session::new(&graph);
    execute_ok(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &registry,
    );

    let statement = parse("MATCH (n:Person) WHERE n.age = 42 RETURN n").unwrap();
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).unwrap();
    let plan = plan(&analyzed, &EmptyProcedureRegistry).unwrap();
    let catalog = LiveIndexCatalog { graph: &graph };
    let ctx = OptimizeContext::default().with_index_catalog(&catalog);
    let optimized = optimize(plan, &ctx);
    let scan = first_scan(&optimized.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(
        scan.access,
        ScanAccess::TypedIndexRange {
            kind: IndexKind::Integer,
            ..
        }
    ));
}

#[test]
fn created_composite_index_is_picked_by_optimizer_in_declaration_order() {
    let graph = SharedGraph::new(GraphId::new(42_110));
    let person = istr("Person");
    let tenant = istr("tenant");
    let kind = istr("kind");
    {
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_composite_property_index_named(
                person,
                [tenant, kind].into_iter().collect(),
                [TypedIndexKind::String, TypedIndexKind::String]
                    .into_iter()
                    .collect(),
                Some(istr("person_tenant_kind_idx")),
            )
            .unwrap();
        txn.commit().unwrap();
    }

    let statement =
        parse("MATCH (n:Person) WHERE n.kind = 'person' AND n.tenant = 't1' RETURN n").unwrap();
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).unwrap();
    let plan = plan(&analyzed, &EmptyProcedureRegistry).unwrap();
    let catalog = LiveIndexCatalog { graph: &graph };
    let ctx = OptimizeContext::default().with_index_catalog(&catalog);
    let optimized = optimize(plan, &ctx);
    let scan = first_scan(&optimized.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::CompositeLookup {
        ref properties,
        ref keys,
        ..
    } = scan.access
    else {
        panic!("expected composite lookup, got {:?}", scan.access);
    };
    assert_eq!(properties, &vec![tenant, kind]);
    assert_eq!(keys[0].0, tenant);
    assert_eq!(keys[1].0, kind);
}

#[test]
fn create_index_with_unknown_kind_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42109));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index('Person', 'age', 'bogus')",
        &mut session,
        &registry,
        &registry,
    );

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn create_index_with_empty_label_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let graph = SharedGraph::new(GraphId::new(42110));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index('', 'age', 'i64')",
        &mut session,
        &registry,
        &registry,
    );

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

#[test]
fn create_index_with_wrong_arity_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "create_index",
        runtime_handle(&registry, "create_index"),
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![param("label", GqlType::String, false)],
    );
    let graph = SharedGraph::new(GraphId::new(42111));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index('Person')",
        &mut session,
        &planning,
        &registry,
    );

    assert_invalid_argument(err);
}

#[test]
fn create_index_with_non_string_label_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "create_index",
        runtime_handle(&registry, "create_index"),
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![
            param("label", GqlType::Integer, false),
            param("property", GqlType::String, false),
            param("kind", GqlType::String, false),
        ],
    );
    let graph = SharedGraph::new(GraphId::new(42112));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index(42, 'age', 'i64')",
        &mut session,
        &planning,
        &registry,
    );

    assert_invalid_argument(err);
}

#[test]
fn create_index_with_null_arg_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "create_index",
        runtime_handle(&registry, "create_index"),
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![
            param("label", GqlType::String, true),
            param("property", GqlType::String, false),
            param("kind", GqlType::String, false),
        ],
    );
    let graph = SharedGraph::new(GraphId::new(42113));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index(null, 'age', 'i64')",
        &mut session,
        &planning,
        &registry,
    );

    assert_invalid_argument(err);
}

#[test]
fn drop_index_with_wrong_arity_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "drop_index",
        runtime_handle(&registry, "drop_index"),
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![param("label", GqlType::String, false)],
    );
    let graph = SharedGraph::new(GraphId::new(42114));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.drop_index('Person')",
        &mut session,
        &planning,
        &registry,
    );

    assert_invalid_argument(err);
}

#[test]
fn drop_index_with_non_string_label_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "drop_index",
        runtime_handle(&registry, "drop_index"),
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![
            param("label", GqlType::Integer, false),
            param("property", GqlType::String, false),
        ],
    );
    let graph = SharedGraph::new(GraphId::new(42115));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.drop_index(42, 'age')",
        &mut session,
        &planning,
        &registry,
    );

    assert_invalid_argument(err);
}

#[test]
fn drop_index_with_null_arg_returns_invalid_argument() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "drop_index",
        runtime_handle(&registry, "drop_index"),
        ProcedureMutability::SchemaWrite,
        ProcedureTier::Mutation,
        vec![
            param("label", GqlType::String, true),
            param("property", GqlType::String, false),
        ],
    );
    let graph = SharedGraph::new(GraphId::new(42116));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.drop_index(null, 'age')",
        &mut session,
        &planning,
        &registry,
    );

    assert_invalid_argument(err);
}

#[test]
fn create_index_metadata_is_mutation_tier_schema_write() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let metadata = registry
        .lookup(&[istr("selene"), istr("create_index")])
        .unwrap();

    assert_eq!(metadata.tier, ProcedureTier::Mutation);
    assert_eq!(metadata.mutability, ProcedureMutability::SchemaWrite);
    assert_eq!(metadata.signature.parameters.len(), 3);
    assert!(metadata.output_schema.columns.is_empty());
}

#[test]
fn create_index_graph_context_reports_tier_mismatch() {
    let registry = ProcedurePackRegistry::with_builtins().unwrap();
    let planning = PlanningRegistry::metadata(
        "create_index",
        runtime_handle(&registry, "create_index"),
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![
            param("label", GqlType::String, false),
            param("property", GqlType::String, false),
            param("kind", GqlType::String, false),
        ],
    );
    let graph = SharedGraph::new(GraphId::new(42117));
    let mut session = selene_gql::Session::new(&graph);

    let err = execute_err(
        "CALL selene.create_index('Person', 'age', 'i64')",
        &mut session,
        &planning,
        &registry,
    );

    assert!(matches!(
        err,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::TierMismatch {
                expected: ProcedureTier::Mutation,
                actual: ProcedureTier::Graph,
            },
            ..
        }
    ));
}

fn assert_invalid_argument(error: selene_gql::ExecutorError) {
    assert!(matches!(
        error,
        selene_gql::ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

fn first_scan(tree: &JoinTree) -> Option<&NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. } => first_scan(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan(left).or_else(|| first_scan(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

struct LiveIndexCatalog<'a> {
    graph: &'a SharedGraph,
}

impl IndexCatalog for LiveIndexCatalog<'_> {
    fn typed_index(
        &self,
        target: IndexTarget,
        label: IStr,
        property: IStr,
    ) -> Option<TypedIndexLookup> {
        if target != IndexTarget::Node {
            return None;
        }
        let kind = self
            .graph
            .read()
            .property_index_for(&label, &property)?
            .kind();
        Some(TypedIndexLookup::new(
            IndexHandle::new(1),
            index_kind_from(kind),
        ))
    }

    fn label_index(&self, _target: IndexTarget, _label: IStr) -> Option<IndexHandle> {
        None
    }

    fn composite_index(
        &self,
        target: IndexTarget,
        label: IStr,
        properties: &[IStr],
    ) -> Option<CompositeIndexHandle> {
        if target != IndexTarget::Node {
            return None;
        }
        let mut canonical = properties.to_vec();
        canonical.sort_unstable();
        let graph = self.graph.read();
        let entry = graph.composite_property_index_entry_for(&label, &canonical)?;
        let kinds = entry.kinds();
        // BRIEF-154 §B.2 (F7/F17): per-component IndexKind enables parameter-aware
        // composite probes. Map storage-level TypedIndexKind to the optimizer's
        // IndexKind for each declared property, in declaration order.
        let properties: Vec<(IStr, IndexKind)> = entry
            .declared_properties
            .iter()
            .zip(kinds.iter())
            .map(|(property, kind)| (*property, index_kind_from(*kind)))
            .collect();
        Some(CompositeIndexHandle::new(IndexHandle::new(2), properties))
    }
}

fn index_kind_from(kind: TypedIndexKind) -> IndexKind {
    match kind {
        TypedIndexKind::I64 => IndexKind::Integer,
        TypedIndexKind::F64 => IndexKind::Float,
        TypedIndexKind::String => IndexKind::String,
        TypedIndexKind::Date => IndexKind::Date,
        TypedIndexKind::LocalDateTime => IndexKind::LocalDateTime,
        TypedIndexKind::Uuid => IndexKind::Uuid,
    }
}

#[derive(Debug)]
struct PlanningRegistry {
    metadata: HashMap<Box<[IStr]>, ProcedureMetadata>,
}

impl PlanningRegistry {
    fn metadata(
        procedure: &'static str,
        handle: ProcedureHandle,
        mutability: ProcedureMutability,
        tier: ProcedureTier,
        parameters: Vec<ProcedureParameter>,
    ) -> Self {
        let mut metadata = HashMap::new();
        metadata.insert(
            vec![istr("selene"), istr(procedure)].into_boxed_slice(),
            ProcedureMetadata::new(
                handle,
                ProcedureSignature::new(parameters),
                ProcedureOutputSchema {
                    columns: Vec::new(),
                },
                tier,
                mutability,
                None,
            ),
        );
        Self { metadata }
    }
}

impl ProcedureRegistry for PlanningRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        self.metadata.get(name).cloned()
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        unreachable!("planning-only registry is never executed")
    }
}

fn param(name: &str, ty: GqlType, nullable: bool) -> ProcedureParameter {
    ProcedureParameter::new(istr(name), ty, nullable)
}

fn runtime_handle(registry: &ProcedurePackRegistry, procedure: &'static str) -> ProcedureHandle {
    registry
        .lookup(&[istr("selene"), istr(procedure)])
        .expect("with_builtins() registers procedure")
        .handle
}
