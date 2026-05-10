//! Deterministic executor-output summaries for integration snapshots.

use std::{collections::BTreeMap, fmt};

use selene_core::{BindingTableId, EdgeId, NodeId, Path, PathSegment, Record, RecordTyped, Value};
use selene_graph::SeleneGraph;

use crate::runtime::BindingTable;

/// Input accepted by [`executor_summary`].
pub struct ExecutorSummaryInput<'a> {
    /// Binding table produced by statement execution.
    pub table: &'a BindingTable,
    /// Row-order contract for this plan.
    pub row_order: RowOrderPolicy,
    /// Net graph-state delta measured around statement execution.
    pub deltas: NetGraphDelta,
}

/// Row-order policy used by the snapshot renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowOrderPolicy {
    /// Preserve the executor-emitted row order.
    PreserveEmitted,
    /// Sort rows by raw value identity before assigning stable placeholders.
    SortDeterministic,
}

/// Stable executor-output snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutorSnapshot {
    /// Output schema columns.
    pub schema: Vec<SnapshotColumn>,
    /// Rendered output rows.
    pub rows: Vec<Vec<String>>,
    /// Net graph-state delta.
    pub deltas: NetGraphDelta,
}

/// One rendered output-schema column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotColumn {
    /// Column display name.
    pub name: String,
    /// Analyzer type cell rendered with debug formatting.
    pub ty: String,
}

/// Net graph-state delta between two published snapshots.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetGraphDelta {
    /// Difference in alive node count.
    pub node_count_delta: i64,
    /// Difference in alive edge count.
    pub edge_count_delta: i64,
    /// Whether the bound graph type changed.
    pub schema_changed: bool,
}

impl NetGraphDelta {
    /// Compute the net graph delta between two snapshots.
    #[must_use]
    pub fn between(before: &SeleneGraph, after: &SeleneGraph) -> Self {
        Self {
            node_count_delta: after.node_count() as i64 - before.node_count() as i64,
            edge_count_delta: after.edge_count() as i64 - before.edge_count() as i64,
            schema_changed: before.meta.bound_type != after.meta.bound_type,
        }
    }
}

/// Summarize a binding table and graph delta for insta snapshot assertions.
#[must_use]
pub fn executor_summary(input: &ExecutorSummaryInput<'_>) -> ExecutorSnapshot {
    let schema = input
        .table
        .schema()
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| SnapshotColumn {
            name: column
                .name
                .map(|name| name.as_str().to_owned())
                .or_else(|| {
                    column
                        .hidden
                        .map(|hidden| format!("$hidden_{}", hidden.get()))
                })
                .unwrap_or_else(|| format!("$col_{index}")),
            ty: format!("{:?}", column.ty),
        })
        .collect();

    let mut rows = input.table.rows().iter().collect::<Vec<_>>();
    if input.row_order == RowOrderPolicy::SortDeterministic {
        rows.sort_by_key(|row| raw_row_key(row.values()));
    }

    let mut placeholders = PlaceholderTable::default();
    let rows = rows
        .into_iter()
        .map(|row| {
            row.values()
                .iter()
                .map(|value| render_value(value, &mut placeholders))
                .collect()
        })
        .collect();

    ExecutorSnapshot {
        schema,
        rows,
        deltas: input.deltas,
    }
}

impl fmt::Display for ExecutorSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "schema:")?;
        if self.schema.is_empty() {
            writeln!(formatter, "- <empty>")?;
        } else {
            for column in &self.schema {
                writeln!(formatter, "- {} :: {}", column.name, column.ty)?;
            }
        }
        writeln!(formatter, "rows:")?;
        if self.rows.is_empty() {
            writeln!(formatter, "- <empty>")?;
        } else {
            for row in &self.rows {
                writeln!(formatter, "- [{}]", row.join(", "))?;
            }
        }
        write!(
            formatter,
            "delta: nodes={:+} edges={:+} schema_changed={}",
            self.deltas.node_count_delta, self.deltas.edge_count_delta, self.deltas.schema_changed
        )
    }
}

#[derive(Default)]
struct PlaceholderTable {
    nodes: BTreeMap<NodeId, usize>,
    edges: BTreeMap<EdgeId, usize>,
    tables: BTreeMap<BindingTableId, usize>,
}

impl PlaceholderTable {
    fn node(&mut self, id: NodeId) -> String {
        let next = self.nodes.len();
        format!("#node_{}", *self.nodes.entry(id).or_insert(next))
    }

    fn edge(&mut self, id: EdgeId) -> String {
        let next = self.edges.len();
        format!("#edge_{}", *self.edges.entry(id).or_insert(next))
    }

    fn table(&mut self, id: BindingTableId) -> String {
        let next = self.tables.len();
        format!("#table_{}", *self.tables.entry(id).or_insert(next))
    }
}

fn raw_row_key(values: &[Value]) -> String {
    raw_sequence_key("row:[", "]", values.iter().map(raw_value_key))
}

fn raw_value_key(value: &Value) -> String {
    match value {
        Value::Bool(value) => format!("bool:{value}"),
        Value::Int(value) => format!("int:{value:+020}"),
        Value::Uint(value) => format!("uint:{value:020}"),
        Value::Int128(value) => format!("int128:{value:+040}"),
        Value::Uint128(value) => format!("uint128:{value:040}"),
        Value::Float(value) => format!("float:{:016x}", value.to_bits()),
        Value::Float32(value) => format!("float32:{:08x}", value.to_bits()),
        Value::Decimal(value) => format!("decimal:{value}"),
        Value::String(value) => format!("string:{}", value.as_str()),
        Value::Bytes(value) => format!("bytes:{}", hex_bytes(value)),
        Value::List(values) => raw_sequence_key("list:[", "]", values.iter().map(raw_value_key)),
        Value::Record(record) => raw_record_key(record),
        Value::RecordTyped(record) => raw_typed_record_key(record),
        Value::Path(path) => raw_path_key(path),
        Value::NodeRef(id) => format!("node:{:020}", id.get()),
        Value::EdgeRef(id) => format!("edge:{:020}", id.get()),
        Value::GraphRef(id) => format!("graph:{:020}", id.get()),
        Value::TableRef(id) => format!("table:{:020}", id.get()),
        Value::ZonedDateTime(value) => format!("zoned_datetime:{value:?}"),
        Value::LocalDateTime(value) => format!("local_datetime:{value}"),
        Value::Date(value) => format!("date:{value}"),
        Value::ZonedTime(value) => format!("zoned_time:{value:?}"),
        Value::LocalTime(value) => format!("local_time:{value}"),
        Value::Duration(value) => format!("duration:{value:?}"),
        Value::Extended { type_id, payload } => {
            format!("extended:{}:{}", type_id.0, hex_bytes(payload))
        }
        Value::Null => "null".to_owned(),
        Value::Uuid(value) => format!("uuid:{value}"),
        _ => "<value::unknown>".to_owned(),
    }
}

fn raw_record_key(record: &Record) -> String {
    match record {
        Record::Open(fields) => {
            let mut rendered = fields
                .iter()
                .map(|(key, value)| {
                    raw_sequence_key(
                        "field:",
                        "",
                        [key.as_str().to_owned(), raw_value_key(value)],
                    )
                })
                .collect::<Vec<_>>();
            rendered.sort();
            raw_sequence_key("record:{", "}", rendered)
        }
        _ => "<record::unknown>".to_owned(),
    }
}

fn raw_typed_record_key(record: &RecordTyped) -> String {
    let values = record.values.iter().map(|value| {
        value.as_ref().map_or_else(
            || "none".to_owned(),
            |value| format!("some:{}", raw_value_key(value)),
        )
    });
    raw_sequence_key(
        &format!("record_typed:{}:[", record.type_id.get()),
        "]",
        values,
    )
}

fn raw_path_key(path: &Path) -> String {
    raw_sequence_key(
        &format!("path:{}:{}:[", path.graph.get(), path.start.get()),
        "]",
        path.segments.iter().map(raw_path_segment_key),
    )
}

fn raw_path_segment_key(segment: &PathSegment) -> String {
    format!(
        "segment:{}:{:?}:{}",
        segment.edge.get(),
        segment.direction,
        segment.node.get()
    )
}

fn raw_sequence_key(
    prefix: &str,
    suffix: &str,
    elements: impl IntoIterator<Item = String>,
) -> String {
    let mut output = String::from(prefix);
    for element in elements {
        append_length_prefixed(&mut output, &element);
    }
    output.push_str(suffix);
    output
}

fn append_length_prefixed(output: &mut String, value: &str) {
    output.push_str(&value.len().to_string());
    output.push(':');
    output.push_str(value);
}

fn render_value(value: &Value, placeholders: &mut PlaceholderTable) -> String {
    match value {
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Uint(value) => format!("{value}u"),
        Value::Int128(value) => format!("{value}i128"),
        Value::Uint128(value) => format!("{value}u128"),
        Value::Float(value) => format!("{value:?}"),
        Value::Float32(value) => format!("{value:?}f32"),
        Value::Decimal(value) => value.to_string(),
        Value::String(value) => format!("\"{}\"", value.as_str()),
        Value::Bytes(value) => format!("0x{}", hex_bytes(value)),
        Value::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| render_value(value, placeholders))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Record(record) => render_record(record, placeholders),
        Value::RecordTyped(record) => render_typed_record(record, placeholders),
        Value::Path(path) => render_path(path, placeholders),
        Value::NodeRef(id) => placeholders.node(*id),
        Value::EdgeRef(id) => placeholders.edge(*id),
        Value::GraphRef(id) => format!("GraphId({})", id.get()),
        Value::TableRef(id) => placeholders.table(*id),
        Value::ZonedDateTime(value) => format!("{value:?}"),
        Value::LocalDateTime(value) => value.to_string(),
        Value::Date(value) => value.to_string(),
        Value::ZonedTime(value) => format!("{value:?}"),
        Value::LocalTime(value) => value.to_string(),
        Value::Duration(value) => format!("{value:?}"),
        Value::Extended { type_id, payload } => {
            format!("<extended:{}:{}>", type_id.0, hex_bytes(payload))
        }
        Value::Null => "NULL".to_owned(),
        Value::Uuid(value) => value.to_string(),
        _ => "<value::unknown>".to_owned(),
    }
}

fn render_record(record: &Record, placeholders: &mut PlaceholderTable) -> String {
    match record {
        Record::Open(fields) => {
            let mut rendered = fields
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", key.as_str(), render_value(value, placeholders))
                })
                .collect::<Vec<_>>();
            rendered.sort();
            format!("{{{}}}", rendered.join(", "))
        }
        _ => "<record::unknown>".to_owned(),
    }
}

fn render_typed_record(record: &RecordTyped, placeholders: &mut PlaceholderTable) -> String {
    let values = record
        .values
        .iter()
        .map(|value| {
            value
                .as_ref()
                .map_or("NULL".to_owned(), |value| render_value(value, placeholders))
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("RecordType({})[{values}]", record.type_id.get())
}

fn render_path(path: &Path, placeholders: &mut PlaceholderTable) -> String {
    let start = placeholders.node(path.start);
    let segments = path
        .segments
        .iter()
        .map(|segment| {
            format!(
                "{}:{:?}:{}",
                placeholders.edge(segment.edge),
                segment.direction,
                placeholders.node(segment.node)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Path(graph={}, start={start}, segments=[{segments}])",
        path.graph.get(),
    )
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use selene_core::{NodeId, Value, intern_with_admission};

    use crate::{
        Binding, BindingTable, BindingTableColumn, BindingTableSchema, analyze::AnalyzedType,
    };

    use super::*;

    #[test]
    fn unordered_summary_sorts_before_assigning_placeholders() {
        let schema = BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: None,
                hidden: None,
                ty: AnalyzedType::Dynamic,
            }],
        };
        let lhs = BindingTable::new(
            schema.clone(),
            vec![
                Binding::new([Value::NodeRef(NodeId::new(2))]),
                Binding::new([Value::NodeRef(NodeId::new(1))]),
            ],
        );
        let rhs = BindingTable::new(
            schema,
            vec![
                Binding::new([Value::NodeRef(NodeId::new(1))]),
                Binding::new([Value::NodeRef(NodeId::new(2))]),
            ],
        );
        assert_eq!(summary_for(&lhs), summary_for(&rhs));
    }

    #[test]
    fn raw_list_keys_are_injective_when_string_elements_contain_commas() {
        let lhs = Value::List(vec![
            Value::String(intern_with_admission("a").expect("test string interns").0),
            Value::String(
                intern_with_admission("b,string:c")
                    .expect("test string interns")
                    .0,
            ),
        ]);
        let rhs = Value::List(vec![
            Value::String(
                intern_with_admission("a,string:b")
                    .expect("test string interns")
                    .0,
            ),
            Value::String(intern_with_admission("c").expect("test string interns").0),
        ]);

        assert_ne!(raw_value_key(&lhs), raw_value_key(&rhs));
    }

    fn summary_for(table: &BindingTable) -> ExecutorSnapshot {
        executor_summary(&ExecutorSummaryInput {
            table,
            row_order: RowOrderPolicy::SortDeterministic,
            deltas: NetGraphDelta::default(),
        })
    }
}
