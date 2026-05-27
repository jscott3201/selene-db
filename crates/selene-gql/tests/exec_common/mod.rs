#![allow(dead_code, missing_docs)]

use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, JoinTree, NodeOrEdgeScan, PatternPlan, ScanAccess,
    TxContext, analyze, execute_pattern as execute_pattern_plan, execute_pipeline, optimize, parse,
    plan,
};
use selene_graph::{SharedGraph, TypedIndexKind};
use selene_testing::MockIndexCatalog;

pub const LARGE_COUNTER_A: i64 = 9_007_199_254_740_992;
pub const LARGE_COUNTER_B: i64 = 9_007_199_254_740_993;

pub struct ExecFixture {
    pub graph: SharedGraph,
    pub person: IStr,
    pub sensor: IStr,
    pub counter: IStr,
    pub age: IStr,
    pub count: IStr,
    pub name: IStr,
    pub email: IStr,
    pub tenant: IStr,
    pub kind: IStr,
    pub score: IStr,
}

impl ExecFixture {
    pub fn build() -> Self {
        let person = istr("Person");
        let sensor = istr("Sensor");
        let counter = istr("Counter");
        let knows = istr("KNOWS");
        let age = istr("age");
        let count = istr("count");
        let name = istr("name");
        let email = istr("email");
        let tenant = istr("tenant");
        let kind = istr("kind");
        let score = istr("score");
        let graph = SharedGraph::new(GraphId::new(31));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            let alice = mutator
                .create_node(
                    LabelSet::single(person),
                    props([
                        (age, Value::Int(30)),
                        (name, Value::String(istr("Alice"))),
                        (email, Value::String(istr("alice@example.com"))),
                        (tenant, Value::String(istr("t1"))),
                        (kind, Value::String(istr("person"))),
                        (score, Value::Int(7)),
                    ]),
                )
                .expect("alice inserts");
            let bob = mutator
                .create_node(
                    LabelSet::single(person),
                    props([
                        (age, Value::Int(42)),
                        (name, Value::String(istr("Bob"))),
                        (email, Value::String(istr("bob@example.com"))),
                        (tenant, Value::String(istr("t1"))),
                        (kind, Value::String(istr("person"))),
                        (score, Value::Int(3)),
                    ]),
                )
                .expect("bob inserts");
            mutator
                .create_node(
                    LabelSet::single(person),
                    props([
                        (age, Value::Int(55)),
                        (name, Value::String(istr("Cara"))),
                        (email, Value::String(istr("cara@example.com"))),
                        (tenant, Value::String(istr("t2"))),
                        (kind, Value::String(istr("person"))),
                        (score, Value::Int(9)),
                    ]),
                )
                .expect("cara inserts");
            let sensor_node = mutator
                .create_node(
                    LabelSet::single(sensor),
                    props([(age, Value::Int(5)), (score, Value::Int(99))]),
                )
                .expect("sensor inserts");
            mutator
                .create_node(
                    LabelSet::single(counter),
                    props([(count, Value::Int(LARGE_COUNTER_A))]),
                )
                .expect("counter A inserts");
            mutator
                .create_node(
                    LabelSet::single(counter),
                    props([(count, Value::Int(LARGE_COUNTER_B))]),
                )
                .expect("counter B inserts");
            mutator
                .create_edge(knows, alice, bob, props([(score, Value::Int(1))]))
                .expect("edge inserts");
            mutator
                .create_edge(knows, bob, sensor_node, props([(score, Value::Int(2))]))
                .expect("edge inserts");
            txn.commit().expect("fixture commits");
        }
        graph
            .create_property_index(person, age, TypedIndexKind::I64)
            .expect("age index builds");
        graph
            .create_property_index(person, email, TypedIndexKind::String)
            .expect("email index builds");
        graph
            .create_property_index(person, tenant, TypedIndexKind::String)
            .expect("tenant index builds");
        graph
            .create_property_index(person, kind, TypedIndexKind::String)
            .expect("kind index builds");
        Self {
            graph,
            person,
            sensor,
            counter,
            age,
            count,
            name,
            email,
            tenant,
            kind,
            score,
        }
    }

    pub fn context_caps<'a>(&'a self, plan: &'a ExecutionPlan) -> TxContext<'a, 'a> {
        TxContext::read_only(
            self.graph.read(),
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            self.graph.index_providers(),
        )
        .with_plan_metadata(&plan.expr_ids, &plan.subqueries)
    }

    pub fn index_catalog(&self) -> MockIndexCatalog {
        MockIndexCatalog::new()
            .with_node_label_index(self.person)
            .with_node_typed_index(self.person, self.age, selene_gql::IndexKind::Integer)
            .with_node_typed_index(self.person, self.email, selene_gql::IndexKind::String)
            .with_node_typed_index(self.person, self.tenant, selene_gql::IndexKind::String)
            .with_node_typed_index(self.person, self.kind, selene_gql::IndexKind::String)
            .with_node_composite_index(
                self.person,
                vec![
                    (self.tenant, selene_gql::IndexKind::String),
                    (self.kind, selene_gql::IndexKind::String),
                ],
            )
    }
}

pub fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

pub fn planned(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

pub fn optimized(source: &str, catalog: &MockIndexCatalog) -> ExecutionPlan {
    let plan = planned(source);
    let ctx = selene_gql::OptimizeContext::default().with_index_catalog(catalog);
    optimize(plan, &ctx)
}

pub fn first_scan(tree: &JoinTree) -> Option<&NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. } | JoinTree::Repeat { child, .. } => first_scan(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan(left).or_else(|| first_scan(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

pub fn first_scan_mut(tree: &mut JoinTree) -> Option<&mut NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. } | JoinTree::Repeat { child, .. } => first_scan_mut(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan_mut(left).or_else(|| first_scan_mut(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

pub fn execute_pattern(pattern: &PatternPlan, ctx: &TxContext<'_, '_>) -> selene_gql::BindingTable {
    execute_pattern_plan(pattern, ctx).expect("pattern executes")
}

pub fn execute_read(source: &str) -> selene_gql::BindingTable {
    let fixture = ExecFixture::build();
    let plan = planned(source);
    execute_plan(&fixture, &plan).expect("read pipeline executes")
}

pub fn execute_optimized_read(source: &str) -> selene_gql::BindingTable {
    let fixture = ExecFixture::build();
    let plan = optimized(source, &fixture.index_catalog());
    execute_plan(&fixture, &plan).expect("read pipeline executes")
}

pub fn execute_read_result(
    source: &str,
) -> Result<selene_gql::BindingTable, selene_gql::ExecutorError> {
    let fixture = ExecFixture::build();
    let plan = planned(source);
    execute_plan(&fixture, &plan)
}

pub fn execute_plan(
    fixture: &ExecFixture,
    plan: &ExecutionPlan,
) -> Result<selene_gql::BindingTable, selene_gql::ExecutorError> {
    let mut ctx = fixture.context_caps(plan);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx)
    } else {
        selene_gql::BindingTable::new(
            selene_gql::BindingTableSchema {
                columns: Vec::new(),
            },
            vec![selene_gql::Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx)
}

pub fn node_ids(table: &selene_gql::BindingTable) -> Vec<u64> {
    table
        .rows()
        .iter()
        .filter_map(|row| match row.values().first() {
            Some(Value::NodeRef(id)) => Some(id.get()),
            _ => None,
        })
        .collect()
}

pub fn edge_ids(table: &selene_gql::BindingTable) -> Vec<u64> {
    table
        .rows()
        .iter()
        .filter_map(|row| match row.values().first() {
            Some(Value::EdgeRef(id)) => Some(id.get()),
            _ => None,
        })
        .collect()
}

pub fn column_values(table: &selene_gql::BindingTable, name: &str) -> Vec<Value> {
    let index = table
        .schema()
        .columns
        .iter()
        .position(|column| column.name.is_some_and(|column| column.as_str() == name))
        .expect("column exists");
    table
        .rows()
        .iter()
        .map(|row| row.get(index).cloned().unwrap_or(Value::Null))
        .collect()
}

pub fn node_ids_for(table: &selene_gql::BindingTable, name: &str) -> Vec<Option<u64>> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::NodeRef(id) => Some(id.get()),
            Value::Null => None,
            other => panic!("expected node or null, got {other:?}"),
        })
        .collect()
}

pub fn edge_ids_for(table: &selene_gql::BindingTable, name: &str) -> Vec<Option<u64>> {
    column_values(table, name)
        .into_iter()
        .map(|value| match value {
            Value::EdgeRef(id) => Some(id.get()),
            Value::Null => None,
            other => panic!("expected edge or null, got {other:?}"),
        })
        .collect()
}

pub fn set_first_scan_access(pattern: &mut PatternPlan, access: ScanAccess) {
    first_scan_mut(&mut pattern.join_tree)
        .expect("pattern has a scan")
        .access = access;
}

pub fn props<const N: usize>(pairs: [(IStr, Value); N]) -> PropertyMap {
    PropertyMap::from_pairs(pairs).expect("test properties fit caps")
}

#[allow(dead_code)]
pub fn empty_graph_context(caps: &selene_gql::ImplDefinedCaps) -> TxContext<'_, '_> {
    TxContext::read_only(
        Arc::new(selene_graph::SeleneGraph::new(GraphId::new(999))),
        caps,
        &EmptyProcedureRegistry,
        &[],
    )
}

#[allow(dead_code)]
pub fn node_id(raw: u64) -> Value {
    Value::NodeRef(NodeId::new(raw))
}
