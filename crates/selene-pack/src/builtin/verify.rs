//! `selene.verify` built-in.

use std::sync::Arc;

use selene_core::{EdgeId, IStr, NodeId, Value, intern_with_admission};
use selene_gql::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureError, ProcedureMutability,
    ProcedureResult, ProcedureTier,
};
use selene_graph::{AdjacencyEntry, NotNanF64, SeleneGraph, TypedIndex};

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};

static VERIFY_PARAMS: [StaticParameter; 1] =
    [StaticParameter::new("deep", GqlType::Boolean, false)
        .with_description("Whether to run slower deep integrity checks.")
        .with_default_doc("false")
        .with_default(ProcedureDefaultValue::Boolean(false))];

static VERIFY_OUTPUTS: [StaticOutputColumn; 3] = [
    StaticOutputColumn::new("check", GqlType::String).with_description("Integrity check name."),
    StaticOutputColumn::new("status", GqlType::String)
        .with_description("Integrity check status: ok or inconsistent."),
    StaticOutputColumn::new("detail", GqlType::String)
        .with_description("Human-readable integrity check detail."),
];

/// Built-in read-only graph integrity-check procedure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeleneVerify;

impl BuiltInMetadata for SeleneVerify {
    fn name(&self) -> &'static [&'static str] {
        &["selene", "verify"]
    }

    fn description(&self) -> &'static str {
        "Integrity check against graph invariants."
    }

    fn since_version(&self) -> &'static str {
        "1.1.0"
    }

    fn tier(&self) -> ProcedureTier {
        ProcedureTier::Graph
    }

    fn mutability(&self) -> ProcedureMutability {
        ProcedureMutability::Read
    }

    fn signature_static(&self) -> &'static [StaticParameter] {
        &VERIFY_PARAMS
    }

    fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
        &VERIFY_OUTPUTS
    }
}

impl GraphProcedureBuiltIn for SeleneVerify {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let deep = deep_arg(args)?;

        verify_snapshot(ctx.snapshot(), deep)
    }
}

fn deep_arg(args: &[Value]) -> Result<bool, ProcedureError> {
    match args {
        [] => Ok(false),
        [Value::Bool(value)] => Ok(*value),
        [_] => Err(ProcedureError::InvalidArgument {
            detail: "selene.verify deep must be a BOOLEAN".to_owned(),
        }),
        _ => Err(ProcedureError::InvalidArgument {
            detail: "selene.verify expects at most 1 argument".to_owned(),
        }),
    }
}

fn verify_snapshot(snapshot: &SeleneGraph, deep: bool) -> Result<ProcedureResult, ProcedureError> {
    let mut rows = vec![
        check_row(
            "label_index_cardinality",
            check_label_index_cardinality(snapshot),
        )?,
        check_row(
            "property_index_coverage",
            check_property_index_coverage(snapshot),
        )?,
        check_row("adjacency_symmetry", check_adjacency_symmetry(snapshot))?,
        check_row(
            "edge_endpoint_liveness",
            check_edge_endpoint_liveness(snapshot),
        )?,
    ];
    if deep {
        rows.push(check_row(
            "typed_index_value_range",
            check_typed_index_value_range(snapshot),
        )?);
        rows.push(check_row(
            "roaring_bitmap_density",
            check_roaring_bitmap_density(snapshot),
        )?);
    }
    Ok(ProcedureResult { rows })
}

fn check_row(check: &'static str, result: CheckResult) -> Result<Vec<Value>, ProcedureError> {
    Ok(vec![
        static_string(check)?,
        static_string(if result.issues == 0 {
            "ok"
        } else {
            "inconsistent"
        })?,
        Value::ExternalString(Arc::from(result.detail)),
    ])
}

fn check_label_index_cardinality(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut indexed_rows = 0_u64;
    let mut expected_rows = 0_u64;

    for (label, bitmap) in &snapshot.idx_label {
        indexed_rows += bitmap.len();
        for row in bitmap {
            if !live_node_row(snapshot, row) {
                issues += 1;
                continue;
            }
            let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
                issues += 1;
                continue;
            };
            if !labels.contains(label) {
                issues += 1;
            }
        }
    }

    for row in snapshot.live_nodes() {
        let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
            issues += 1;
            continue;
        };
        for label in labels.iter() {
            expected_rows += 1;
            if !snapshot
                .nodes_with_label(label)
                .is_some_and(|bitmap| bitmap.contains(row))
            {
                issues += 1;
            }
        }
    }

    if indexed_rows != expected_rows {
        issues += indexed_rows.abs_diff(expected_rows) as usize;
    }
    CheckResult::new(
        issues,
        format!(
            "indexed label rows={indexed_rows}; expected label rows={expected_rows}; issues={issues}"
        ),
    )
}

fn check_property_index_coverage(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut indexed_rows = 0_u64;
    let mut expected_rows = 0_u64;

    for ((label, property), entry) in &snapshot.property_index {
        indexed_rows += entry.index.cardinality();
        for row in snapshot.live_nodes() {
            let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
                issues += 1;
                continue;
            };
            if !labels.contains(label) {
                continue;
            }
            let Some(properties) = snapshot.node_store.properties.get(row as usize) else {
                issues += 1;
                continue;
            };
            let Some(value) = properties.get(property) else {
                continue;
            };
            if let Some(bitmap) = snapshot.nodes_with_property_eq(label, property, value) {
                expected_rows += 1;
                if !bitmap.contains(row) {
                    issues += 1;
                }
            }
        }
    }

    if indexed_rows != expected_rows {
        issues += indexed_rows.abs_diff(expected_rows) as usize;
    }
    CheckResult::new(
        issues,
        format!(
            "indexed property rows={indexed_rows}; expected property rows={expected_rows}; issues={issues}"
        ),
    )
}

fn check_adjacency_symmetry(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut outgoing_edges = 0_usize;
    let mut incoming_edges = 0_usize;
    let mut live_edges = 0_usize;

    for (source, entry) in &snapshot.adjacency_out {
        for edge in entry.iter() {
            outgoing_edges += 1;
            if !snapshot.is_node_alive(*source) || !snapshot.is_node_alive(edge.neighbor) {
                issues += 1;
            }
            match expected_edge(snapshot, edge.edge_id) {
                Some((actual_source, actual_target, actual_label)) => {
                    if actual_source != *source || actual_target != edge.neighbor {
                        issues += 1;
                    }
                    if actual_label != edge.label {
                        issues += 1;
                    }
                }
                None => {
                    issues += 1;
                }
            }
            if !snapshot
                .incoming_edges(edge.neighbor)
                .is_some_and(|incoming| {
                    incoming.iter().any(|candidate| {
                        candidate.edge_id == edge.edge_id
                            && candidate.neighbor == *source
                            && candidate.label == edge.label
                    })
                })
            {
                issues += 1;
            }
        }
    }

    for (target, entry) in &snapshot.adjacency_in {
        for edge in entry.iter() {
            incoming_edges += 1;
            if !snapshot.is_node_alive(*target) || !snapshot.is_node_alive(edge.neighbor) {
                issues += 1;
            }
            match expected_edge(snapshot, edge.edge_id) {
                Some((actual_source, actual_target, actual_label)) => {
                    if actual_source != edge.neighbor || actual_target != *target {
                        issues += 1;
                    }
                    if actual_label != edge.label {
                        issues += 1;
                    }
                }
                None => {
                    issues += 1;
                }
            }
            if !snapshot
                .outgoing_edges(edge.neighbor)
                .is_some_and(|outgoing| {
                    outgoing.iter().any(|candidate| {
                        candidate.edge_id == edge.edge_id
                            && candidate.neighbor == *target
                            && candidate.label == edge.label
                    })
                })
            {
                issues += 1;
            }
        }
    }

    for row in &snapshot.edge_store.alive {
        live_edges += 1;
        let edge_id = EdgeId::new(u64::from(row) + 1);
        let Some((source, target, label)) = expected_edge(snapshot, edge_id) else {
            issues += 1;
            continue;
        };
        if !adjacency_entry_contains(snapshot.outgoing_edges(source), target, edge_id, label) {
            issues += 1;
        }
        if !adjacency_entry_contains(snapshot.incoming_edges(target), source, edge_id, label) {
            issues += 1;
        }
    }

    if outgoing_edges != live_edges {
        issues += outgoing_edges.abs_diff(live_edges);
    }
    if incoming_edges != live_edges {
        issues += incoming_edges.abs_diff(live_edges);
    }
    CheckResult::new(
        issues,
        format!(
            "live edges={live_edges}; outgoing adjacency edges={outgoing_edges}; incoming adjacency edges={incoming_edges}; issues={issues}"
        ),
    )
}

fn expected_edge(snapshot: &SeleneGraph, edge_id: EdgeId) -> Option<(NodeId, NodeId, IStr)> {
    let (source, target) = snapshot.edge_endpoints(edge_id)?;
    let label = *snapshot.edge_label(edge_id)?;
    Some((source, target, label))
}

fn adjacency_entry_contains(
    entry: Option<&AdjacencyEntry>,
    neighbor: NodeId,
    edge_id: EdgeId,
    label: IStr,
) -> bool {
    entry.is_some_and(|entry| {
        entry
            .iter()
            .any(|edge| edge.neighbor == neighbor && edge.edge_id == edge_id && edge.label == label)
    })
}

fn check_edge_endpoint_liveness(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut checked = 0_usize;

    for row in &snapshot.edge_store.alive {
        checked += 1;
        let edge_id = EdgeId::new(u64::from(row) + 1);
        match snapshot.edge_endpoints(edge_id) {
            Some((source, target))
                if snapshot.is_node_alive(source) && snapshot.is_node_alive(target) => {}
            _ => {
                issues += 1;
            }
        }
    }

    CheckResult::new(
        issues,
        format!("live edges checked={checked}; issues={issues}"),
    )
}

fn check_typed_index_value_range(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut checked = 0_u64;

    for ((label, property), entry) in &snapshot.property_index {
        for (bucket, row) in typed_index_entries(&entry.index) {
            checked += 1;
            if !indexed_property_row_matches(snapshot, *label, *property, row, bucket) {
                issues += 1;
            }
        }
    }

    CheckResult::new(
        issues,
        format!("typed index rows checked={checked}; issues={issues}"),
    )
}

fn check_roaring_bitmap_density(snapshot: &SeleneGraph) -> CheckResult {
    let mut issues = 0_usize;
    let mut bitmaps = 0_usize;

    for bitmap in snapshot.idx_label.values() {
        bitmaps += 1;
        for row in bitmap {
            if !live_node_row(snapshot, row) {
                issues += 1;
            }
        }
    }
    for bitmap in snapshot.idx_edge_label.values() {
        bitmaps += 1;
        for row in bitmap {
            if !live_edge_row(snapshot, row) {
                issues += 1;
            }
        }
    }
    for entry in snapshot.property_index.values() {
        for (_bucket, row) in typed_index_entries(&entry.index) {
            if !live_node_row(snapshot, row) {
                issues += 1;
            }
        }
    }

    CheckResult::new(
        issues,
        format!("bitmap groups checked={bitmaps}; issues={issues}"),
    )
}

fn typed_index_entries(index: &TypedIndex) -> Vec<(IndexedValue, u32)> {
    let mut entries = Vec::new();
    match index {
        TypedIndex::I64(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::I64(*key), bitmap.iter());
            }
        }
        TypedIndex::F64(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::F64(*key), bitmap.iter());
            }
        }
        TypedIndex::String(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::String(*key), bitmap.iter());
            }
        }
        TypedIndex::Date(index) => {
            for (key, bitmap) in index {
                push_index_entries(&mut entries, IndexedValue::Date(*key), bitmap.iter());
            }
        }
        TypedIndex::LocalDateTime(index) => {
            for (key, bitmap) in index {
                push_index_entries(
                    &mut entries,
                    IndexedValue::LocalDateTime(*key),
                    bitmap.iter(),
                );
            }
        }
        TypedIndex::Uuid(index) => {
            for (key, bitmap) in index {
                push_index_entries(
                    &mut entries,
                    IndexedValue::Uuid(key.to_string()),
                    bitmap.iter(),
                );
            }
        }
    }
    entries
}

fn push_index_entries(
    entries: &mut Vec<(IndexedValue, u32)>,
    key: IndexedValue,
    rows: impl Iterator<Item = u32>,
) {
    entries.extend(rows.map(|row| (key.clone(), row)));
}

fn indexed_property_row_matches(
    snapshot: &SeleneGraph,
    label: IStr,
    property: IStr,
    row: u32,
    bucket: IndexedValue,
) -> bool {
    if !live_node_row(snapshot, row) {
        return false;
    }
    let Some(labels) = snapshot.node_store.labels.get(row as usize) else {
        return false;
    };
    if !labels.contains(&label) {
        return false;
    }
    let Some(properties) = snapshot.node_store.properties.get(row as usize) else {
        return false;
    };
    let Some(value) = properties.get(&property) else {
        return false;
    };
    bucket_matches_value(bucket, value)
}

#[derive(Clone, Debug)]
enum IndexedValue {
    I64(i64),
    F64(NotNanF64),
    String(IStr),
    Date(jiff::civil::Date),
    LocalDateTime(jiff::civil::DateTime),
    Uuid(String),
}

fn bucket_matches_value(bucket: IndexedValue, value: &Value) -> bool {
    match (bucket, value) {
        (IndexedValue::I64(expected), Value::Int(actual)) => expected == *actual,
        (IndexedValue::F64(expected), Value::Float(actual)) => {
            NotNanF64::new(*actual).is_ok_and(|actual| actual == expected)
        }
        (IndexedValue::String(expected), Value::String(actual)) => expected == *actual,
        (IndexedValue::Date(expected), Value::Date(actual)) => expected == *actual,
        (IndexedValue::LocalDateTime(expected), Value::LocalDateTime(actual)) => {
            expected == *actual
        }
        (IndexedValue::Uuid(expected), Value::Uuid(actual)) => expected == actual.to_string(),
        _ => false,
    }
}

fn live_node_row(snapshot: &SeleneGraph, row: u32) -> bool {
    (row as usize) < snapshot.node_store.len() && snapshot.node_store.is_alive(row)
}

fn live_edge_row(snapshot: &SeleneGraph, row: u32) -> bool {
    (row as usize) < snapshot.edge_store.len() && snapshot.edge_store.is_alive(row)
}

fn static_string(value: &'static str) -> Result<Value, ProcedureError> {
    intern_with_admission(value)
        .map(|(value, _was_new)| Value::String(value))
        .map_err(|_err| ProcedureError::Internal {
            detail: "interner cap exhausted during selene.verify".to_owned(),
        })
}

struct CheckResult {
    issues: usize,
    detail: String,
}

impl CheckResult {
    fn new(issues: usize, detail: String) -> Self {
        Self { issues, detail }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::{GraphId, LabelSet, PropertyMap, intern};
    use selene_graph::SharedGraph;

    use super::*;

    fn istr(value: &str) -> IStr {
        intern(value).expect("test string interns")
    }

    fn graph_with_one_indexed_node() -> SeleneGraph {
        let graph = SharedGraph::new(GraphId::new(121_301));
        let label = istr("Person");
        let age = istr("age");
        let mut props = PropertyMap::new();
        props.set(age, Value::Int(42)).unwrap();
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_node(LabelSet::single(label), props)
            .expect("node created");
        txn.mutator()
            .create_property_index(label, age, selene_graph::TypedIndexKind::I64)
            .expect("index created");
        txn.commit().expect("seed commit succeeds");
        graph.read().as_ref().clone()
    }

    fn graph_with_one_edge() -> SeleneGraph {
        let graph = SharedGraph::new(GraphId::new(121_302));
        let label = istr("Person");
        let edge_label = istr("KNOWS");
        let mut txn = graph.begin_write();
        let source = txn
            .mutator()
            .create_node(LabelSet::single(label), PropertyMap::new())
            .expect("source created");
        let target = txn
            .mutator()
            .create_node(LabelSet::single(label), PropertyMap::new())
            .expect("target created");
        txn.mutator()
            .create_edge(edge_label, source, target, PropertyMap::new())
            .expect("edge created");
        txn.commit().expect("seed commit succeeds");
        graph.read().as_ref().clone()
    }

    fn status_for(result: &ProcedureResult, check: &str) -> String {
        let row = result
            .rows
            .iter()
            .find(|row| matches!(&row[0], Value::String(name) if name.as_str() == check))
            .unwrap_or_else(|| panic!("{check} row exists"));
        let Value::String(status) = &row[1] else {
            panic!("{check} status is a string");
        };
        status.as_str().to_owned()
    }

    #[test]
    fn corrupted_label_bitmap_reports_inconsistent_row_without_rebuild() {
        let mut graph = graph_with_one_indexed_node();
        graph
            .idx_label
            .get_mut(&istr("Person"))
            .expect("label index exists")
            .insert(10);

        let result = verify_snapshot(&graph, false).expect("verification rows");
        assert_eq!(
            status_for(&result, "label_index_cardinality"),
            "inconsistent"
        );
    }

    #[test]
    fn property_index_coverage_reports_extra_live_row_bucket() {
        let mut graph = graph_with_one_indexed_node();
        let label = istr("Person");
        let age = istr("age");
        let entry = graph
            .property_index
            .get_mut(&(label, age))
            .expect("property index exists");
        let index = Arc::make_mut(&mut entry.index);
        let TypedIndex::I64(map) = index else {
            panic!("test index is i64");
        };
        map.entry(99).or_default().insert(0);

        let result = verify_snapshot(&graph, false).expect("verification rows");

        assert_eq!(
            status_for(&result, "property_index_coverage"),
            "inconsistent"
        );
    }

    #[test]
    fn adjacency_symmetry_reports_live_edge_missing_from_both_maps() {
        let mut graph = graph_with_one_edge();
        graph.adjacency_out.clear();
        graph.adjacency_in.clear();

        let result = verify_snapshot(&graph, false).expect("verification rows");

        assert_eq!(status_for(&result, "adjacency_symmetry"), "inconsistent");
    }

    #[test]
    fn adjacency_symmetry_reports_label_drift_in_both_maps() {
        let mut graph = graph_with_one_edge();
        let wrong_label = istr("LIKES");
        let source = NodeId::new(1);
        let target = NodeId::new(2);
        graph
            .adjacency_out
            .get_mut(&source)
            .expect("source adjacency exists")
            .edges[0]
            .label = wrong_label;
        graph
            .adjacency_in
            .get_mut(&target)
            .expect("target adjacency exists")
            .edges[0]
            .label = wrong_label;

        let result = verify_snapshot(&graph, false).expect("verification rows");

        assert_eq!(status_for(&result, "adjacency_symmetry"), "inconsistent");
    }

    #[test]
    fn typed_index_value_range_reports_live_row_in_wrong_bucket() {
        let mut graph = graph_with_one_indexed_node();
        let label = istr("Person");
        let age = istr("age");
        let entry = graph
            .property_index
            .get_mut(&(label, age))
            .expect("property index exists");
        let index = Arc::make_mut(&mut entry.index);
        let TypedIndex::I64(map) = index else {
            panic!("test index is i64");
        };
        map.entry(99).or_default().insert(0);

        let result = verify_snapshot(&graph, true).expect("verification rows");

        assert_eq!(
            status_for(&result, "typed_index_value_range"),
            "inconsistent"
        );
    }

    #[test]
    fn deep_check_reports_stale_property_index_bitmap_row() {
        let mut graph = graph_with_one_indexed_node();
        let label = istr("Person");
        let age = istr("age");
        let entry = graph
            .property_index
            .get_mut(&(label, age))
            .expect("property index exists");
        let index = Arc::make_mut(&mut entry.index);
        let TypedIndex::I64(map) = index else {
            panic!("test index is i64");
        };
        map.entry(99).or_default().insert(99);

        let result = verify_snapshot(&graph, true).expect("verification rows");
        assert_eq!(
            status_for(&result, "roaring_bitmap_density"),
            "inconsistent"
        );
    }
}
