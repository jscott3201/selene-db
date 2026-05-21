//! `selene.verify` built-in.

use std::sync::Arc;

use selene_core::{EdgeId, IStr, Value, intern_with_admission};
use selene_gql::{
    GqlType, GraphContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
};
use selene_graph::{SeleneGraph, TypedIndex};

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};

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
        &[]
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
        if !args.is_empty() {
            return Err(ProcedureError::InvalidArgument {
                detail: "selene.verify expects zero arguments".to_owned(),
            });
        }

        verify_snapshot(ctx.snapshot(), false)
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

    if indexed_rows < expected_rows {
        issues += (expected_rows - indexed_rows) as usize;
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

    for (source, entry) in &snapshot.adjacency_out {
        for edge in entry.iter() {
            outgoing_edges += 1;
            if !snapshot.is_node_alive(*source) || !snapshot.is_node_alive(edge.neighbor) {
                issues += 1;
            }
            match snapshot.edge_endpoints(edge.edge_id) {
                Some((actual_source, actual_target))
                    if actual_source == *source && actual_target == edge.neighbor => {}
                _ => {
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
            match snapshot.edge_endpoints(edge.edge_id) {
                Some((actual_source, actual_target))
                    if actual_source == edge.neighbor && actual_target == *target => {}
                _ => {
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

    if outgoing_edges != incoming_edges {
        issues += outgoing_edges.abs_diff(incoming_edges);
    }
    CheckResult::new(
        issues,
        format!(
            "outgoing adjacency edges={outgoing_edges}; incoming adjacency edges={incoming_edges}; issues={issues}"
        ),
    )
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
        for row in typed_index_rows(&entry.index) {
            checked += 1;
            if !indexed_property_row_matches(snapshot, *label, *property, row) {
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
        for row in typed_index_rows(&entry.index) {
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

fn typed_index_rows(index: &TypedIndex) -> Vec<u32> {
    let mut rows = Vec::new();
    match index {
        TypedIndex::I64(index) => {
            for bitmap in index.values() {
                rows.extend(bitmap.iter());
            }
        }
        TypedIndex::F64(index) => {
            for bitmap in index.values() {
                rows.extend(bitmap.iter());
            }
        }
        TypedIndex::String(index) => {
            for bitmap in index.values() {
                rows.extend(bitmap.iter());
            }
        }
        TypedIndex::Date(index) => {
            for bitmap in index.values() {
                rows.extend(bitmap.iter());
            }
        }
        TypedIndex::LocalDateTime(index) => {
            for bitmap in index.values() {
                rows.extend(bitmap.iter());
            }
        }
        TypedIndex::Uuid(index) => {
            for bitmap in index.values() {
                rows.extend(bitmap.iter());
            }
        }
    }
    rows
}

fn indexed_property_row_matches(
    snapshot: &SeleneGraph,
    label: IStr,
    property: IStr,
    row: u32,
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
    snapshot
        .nodes_with_property_eq(&label, &property, value)
        .is_some_and(|bitmap| bitmap.contains(row))
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

    #[test]
    fn corrupted_label_bitmap_reports_inconsistent_row_without_rebuild() {
        let mut graph = graph_with_one_indexed_node();
        graph
            .idx_label
            .get_mut(&istr("Person"))
            .expect("label index exists")
            .insert(10);

        let result = verify_snapshot(&graph, false).expect("verification rows");
        let row = result
            .rows
            .iter()
            .find(|row| matches!(&row[0], Value::String(name) if name.as_str() == "label_index_cardinality"))
            .expect("label-index row exists");

        assert!(matches!(&row[1], Value::String(status) if status.as_str() == "inconsistent"));
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
        let row = result
            .rows
            .iter()
            .find(|row| matches!(&row[0], Value::String(name) if name.as_str() == "roaring_bitmap_density"))
            .expect("deep bitmap row exists");

        assert!(matches!(&row[1], Value::String(status) if status.as_str() == "inconsistent"));
    }
}
