#![allow(missing_docs)]
//! Criterion benches for closed-graph type validation during commit.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::sync::Arc;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    ByteStringType, CharacterStringType, DbString, GraphId, GraphTypeId, LabelDiff, LabelSet,
    PropertyDiff, PropertyMap, PropertyValueType, SchemaChange, Value, db_string,
};
use selene_graph::{
    EdgeEndpointDef, EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, SeleneGraph,
    SharedGraph, ValidationMode,
};
use selene_testing::BenchProfile;

const UPDATE_BATCH: usize = 100;

fn bench_bound_type_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("bound_type_validation");
    group.throughput(Throughput::Elements(UPDATE_BATCH as u64));
    for &scale in BenchProfile::from_env().scales() {
        let simple = simple_graph_type();
        let unique = unique_graph_type();
        let rich = rich_graph_type();
        let unbound_graph = graph_snapshot(scale, None, &[label("SimpleNode")], 3);
        let simple_graph = graph_snapshot(scale, Some(simple), &[label("SimpleNode")], 3);
        let unique_graph = graph_snapshot(scale, Some(unique), &[label("UniqueNode")], 3);
        let rich_labels = (0..10)
            .map(|idx| label(&format!("RichNode{idx}")))
            .collect::<Vec<_>>();
        let rich_graph = graph_snapshot(scale, Some(rich), &rich_labels, 10);
        let descriptor_graph = descriptor_graph_snapshot(scale);

        bench_update_case(&mut group, "unbound_commit", scale, unbound_graph);
        bench_update_case(&mut group, "bound_commit_simple", scale, simple_graph);
        bench_incident_property_update_case(
            &mut group,
            scale,
            incident_graph_snapshot(scale, simple_graph_type()),
        );
        bench_update_case(
            &mut group,
            "bound_commit_unique",
            scale,
            unique_graph.clone(),
        );
        bench_unique_value_update_case(
            &mut group,
            "bound_commit_unique_value_update",
            scale,
            unique_graph,
        );
        bench_update_case(&mut group, "bound_commit_rich", scale, rich_graph.clone());
        bench_schema_change_case(&mut group, scale, rich_graph);
        bench_descriptor_insert_case(&mut group, scale, descriptor_graph.clone());
        bench_descriptor_update_case(&mut group, scale, descriptor_graph);
    }
    group.finish();
}

fn bench_update_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &'static str,
    scale: usize,
    graph: SeleneGraph,
) {
    group.throughput(Throughput::Elements(UPDATE_BATCH as u64));
    group.bench_function(BenchmarkId::new(name, scale), |b| {
        b.iter_batched(
            || SharedGraph::from_graph(graph.clone()),
            |shared| {
                let changes = update_batch(&shared);
                std::hint::black_box((shared, changes))
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_schema_change_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
    graph: SeleneGraph,
) {
    group.throughput(Throughput::Elements(1));
    group.bench_function(BenchmarkId::new("bound_schema_change", scale), |b| {
        b.iter_batched(
            || SharedGraph::from_graph(graph.clone()),
            |shared| {
                let mut txn = shared.begin_write();
                {
                    let mut mutator = txn.mutator();
                    mutator.schema_change(SchemaChange::NodeTypeDropped {
                        graph_type: GraphTypeId::new(1).expect("non-zero graph type id"),
                        name: label("NoopDroppedType"),
                    });
                }
                let changes = txn.commit().expect("schema commit succeeds").changes.len();
                std::hint::black_box((shared, changes))
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_unique_value_update_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &'static str,
    scale: usize,
    graph: SeleneGraph,
) {
    group.throughput(Throughput::Elements(UPDATE_BATCH as u64));
    group.bench_function(BenchmarkId::new(name, scale), |b| {
        b.iter_batched(
            || SharedGraph::from_graph(graph.clone()),
            |shared| {
                let changes = update_unique_batch(&shared);
                std::hint::black_box((shared, changes))
            },
            BatchSize::SmallInput,
        );
    });
}

fn update_batch(shared: &SharedGraph) -> usize {
    let score = label("score");
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for idx in 0..UPDATE_BATCH {
            let diff = PropertyDiff::new([(score.clone(), Value::Int(idx as i64))], [])
                .expect("property diff is valid");
            mutator
                .update_node(
                    selene_core::NodeId::new(idx as u64 + 1),
                    LabelDiff::new([], []).expect("label diff is valid"),
                    diff,
                )
                .expect("node update succeeds");
        }
    }
    txn.commit().expect("commit succeeds").changes.len()
}

fn bench_incident_property_update_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
    graph: SeleneGraph,
) {
    group.throughput(Throughput::Elements(1));
    group.bench_function(
        BenchmarkId::new("bound_commit_incident_property_update", scale),
        |b| {
            b.iter_batched(
                || SharedGraph::from_graph(graph.clone()),
                |shared| {
                    let changes = update_hub_property(&shared);
                    std::hint::black_box((shared, changes))
                },
                BatchSize::SmallInput,
            );
        },
    );
}

fn update_hub_property(shared: &SharedGraph) -> usize {
    let score = label("score");
    let diff = PropertyDiff::new([(score, Value::Int(42))], []).expect("property diff is valid");
    let mut txn = shared.begin_write();
    {
        txn.mutator()
            .update_node(
                selene_core::NodeId::new(1),
                LabelDiff::new([], []).expect("label diff is valid"),
                diff,
            )
            .expect("hub property update succeeds");
    }
    txn.commit().expect("commit succeeds").changes.len()
}

fn update_unique_batch(shared: &SharedGraph) -> usize {
    let name = label("name");
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for idx in 0..UPDATE_BATCH {
            let diff = PropertyDiff::new(
                [(
                    name.clone(),
                    Value::String(label(&format!("updated-node-{idx}"))),
                )],
                [],
            )
            .expect("property diff is valid");
            mutator
                .update_node(
                    selene_core::NodeId::new(idx as u64 + 1),
                    LabelDiff::new([], []).expect("label diff is valid"),
                    diff,
                )
                .expect("node update succeeds");
        }
    }
    txn.commit().expect("commit succeeds").changes.len()
}

fn bench_descriptor_insert_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
    graph: SeleneGraph,
) {
    group.throughput(Throughput::Elements(UPDATE_BATCH as u64));
    group.bench_function(
        BenchmarkId::new("bound_commit_descriptor_insert", scale),
        |b| {
            b.iter_batched(
                || SharedGraph::from_graph(graph.clone()),
                |shared| {
                    let changes = insert_descriptor_batch(&shared, scale);
                    std::hint::black_box((shared, changes))
                },
                BatchSize::SmallInput,
            );
        },
    );
}

fn bench_descriptor_update_case(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
    graph: SeleneGraph,
) {
    group.throughput(Throughput::Elements(UPDATE_BATCH as u64));
    group.bench_function(
        BenchmarkId::new("bound_commit_descriptor_update", scale),
        |b| {
            b.iter_batched(
                || SharedGraph::from_graph(graph.clone()),
                |shared| {
                    let changes = update_descriptor_batch(&shared);
                    std::hint::black_box((shared, changes))
                },
                BatchSize::SmallInput,
            );
        },
    );
}

fn insert_descriptor_batch(shared: &SharedGraph, scale: usize) -> usize {
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for idx in 0..UPDATE_BATCH {
            mutator
                .create_node(
                    LabelSet::single(label("DescriptorNode")),
                    descriptor_properties(scale + idx),
                )
                .expect("descriptor node insert succeeds");
        }
    }
    txn.commit().expect("commit succeeds").changes.len()
}

fn update_descriptor_batch(shared: &SharedGraph) -> usize {
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for idx in 0..UPDATE_BATCH {
            mutator
                .update_node(
                    selene_core::NodeId::new(idx as u64 + 1),
                    LabelDiff::new([], []).expect("label diff is valid"),
                    descriptor_update_diff(idx),
                )
                .expect("descriptor node update succeeds");
        }
    }
    txn.commit().expect("commit succeeds").changes.len()
}

fn graph_snapshot(
    scale: usize,
    type_def: Option<GraphTypeDef>,
    labels: &[DbString],
    property_count: usize,
) -> SeleneGraph {
    let builder = SharedGraph::builder(GraphId::new(1));
    let shared = if let Some(type_def) = type_def {
        builder
            .bound_to(type_def)
            .expect("graph type is valid")
            .build()
            .expect("typed graph builds")
    } else {
        builder.build().expect("unbound graph builds")
    };
    {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            for idx in 0..scale.max(1) {
                let label = labels[idx % labels.len()].clone();
                mutator
                    .create_node(LabelSet::single(label), properties(idx, property_count))
                    .expect("fixture node inserts");
            }
        }
        txn.commit().expect("fixture commit succeeds");
    }
    shared.read().as_ref().clone()
}

fn incident_graph_snapshot(scale: usize, type_def: GraphTypeDef) -> SeleneGraph {
    let shared = SharedGraph::builder(GraphId::new(1))
        .bound_to(type_def)
        .expect("graph type is valid")
        .build()
        .expect("typed graph builds");
    let node_count = scale.max(2);
    {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let hub = mutator
                .create_node(LabelSet::single(label("SimpleNode")), properties(0, 3))
                .expect("hub inserts");
            for idx in 1..node_count {
                let leaf = mutator
                    .create_node(LabelSet::single(label("SimpleNode")), properties(idx, 3))
                    .expect("leaf inserts");
                mutator
                    .create_edge(label("SIMPLE_EDGE"), hub, leaf, properties(idx, 3))
                    .expect("edge inserts");
            }
        }
        txn.commit().expect("fixture commit succeeds");
    }
    shared.read().as_ref().clone()
}

fn descriptor_graph_snapshot(scale: usize) -> SeleneGraph {
    let shared = SharedGraph::builder(GraphId::new(1))
        .bound_to(descriptor_graph_type())
        .expect("graph type is valid")
        .build()
        .expect("typed graph builds");
    {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            for idx in 0..scale.max(UPDATE_BATCH) {
                mutator
                    .create_node(
                        LabelSet::single(label("DescriptorNode")),
                        descriptor_properties(idx),
                    )
                    .expect("descriptor fixture node inserts");
            }
        }
        txn.commit().expect("fixture commit succeeds");
    }
    shared.read().as_ref().clone()
}

fn simple_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: label("bench.simple.graph"),
        node_types: vec![NodeTypeDef {
            name: label("bench.simple.node"),
            key_labels: LabelSet::single(label("SimpleNode")),
            properties: property_defs(3),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![EdgeTypeDef {
            name: label("bench.simple.edge"),
            label: label("SIMPLE_EDGE"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: property_defs(3),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn unique_graph_type() -> GraphTypeDef {
    let mut properties = property_defs(3);
    properties[1].unique = true;
    GraphTypeDef {
        name: label("bench.unique.graph"),
        node_types: vec![NodeTypeDef {
            name: label("bench.unique.node"),
            key_labels: LabelSet::single(label("UniqueNode")),
            properties,
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn rich_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: label("bench.rich.graph"),
        node_types: (0..10)
            .map(|idx| NodeTypeDef {
                name: label(&format!("bench.rich.node{idx}")),
                key_labels: LabelSet::single(label(&format!("RichNode{idx}"))),
                properties: property_defs(10),
                validation_mode: ValidationMode::Strict,
            })
            .collect(),
        edge_types: (0..10)
            .map(|idx| EdgeTypeDef {
                name: label(&format!("bench.rich.edge{idx}")),
                label: label(&format!("RICH_EDGE{idx}")),
                source_node_type: EdgeEndpointDef::NodeType(idx),
                target_node_type: EdgeEndpointDef::NodeType((idx + 1) % 10),
                properties: property_defs(10),
                validation_mode: ValidationMode::Strict,
            })
            .collect(),
    }
}

fn descriptor_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: label("bench.descriptor.graph"),
        node_types: vec![NodeTypeDef {
            name: label("bench.descriptor.node"),
            key_labels: LabelSet::single(label("DescriptorNode")),
            properties: descriptor_property_defs(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn descriptor_property_defs() -> Vec<PropertyTypeDef> {
    let string_type = CharacterStringType::new(1, 64).expect("valid string type");
    let byte_type = ByteStringType::new(1, 64).expect("valid byte type");
    let mut properties = vec![PropertyTypeDef {
        name: property_name(0),
        value_type: PropertyValueType::Int,
        list_element_type: None,
        required: true,
        default: None,
        immutable: false,
        unique: false,
        decimal_type: None,
        character_string_type: None,
        byte_string_type: None,
        record_field_types: None,
    }];
    properties.extend((0..4).map(|idx| PropertyTypeDef {
        name: descriptor_string_property_name(idx),
        value_type: PropertyValueType::String,
        list_element_type: None,
        required: true,
        default: None,
        immutable: false,
        unique: false,
        decimal_type: None,
        character_string_type: Some(string_type),
        byte_string_type: None,
        record_field_types: None,
    }));
    properties.extend((0..4).map(|idx| PropertyTypeDef {
        name: descriptor_bytes_property_name(idx),
        value_type: PropertyValueType::Bytes,
        list_element_type: None,
        required: true,
        default: None,
        immutable: false,
        unique: false,
        decimal_type: None,
        character_string_type: None,
        byte_string_type: Some(byte_type),
        record_field_types: None,
    }));
    properties
}

fn property_defs(count: usize) -> Vec<PropertyTypeDef> {
    (0..count)
        .map(|idx| PropertyTypeDef {
            name: property_name(idx),
            value_type: if idx == 1 {
                PropertyValueType::String
            } else {
                PropertyValueType::Int
            },
            list_element_type: None,
            required: idx < 3,
            default: None,
            immutable: false,
            unique: false,
            decimal_type: None,
            character_string_type: None,
            byte_string_type: None,
            record_field_types: None,
        })
        .collect()
}

fn properties(row: usize, count: usize) -> PropertyMap {
    PropertyMap::from_pairs((0..count).map(|idx| {
        let value = if idx == 1 {
            Value::String(label(&format!("node-{row}")))
        } else {
            Value::Int((row + idx) as i64)
        };
        (property_name(idx), value)
    }))
    .expect("fixture properties fit core caps")
}

fn descriptor_properties(row: usize) -> PropertyMap {
    let string_values = (0..4).map(|idx| {
        (
            descriptor_string_property_name(idx),
            Value::String(label(&format!("node-{row}-text-{idx}"))),
        )
    });
    let byte_values = (0..4).map(|idx| {
        (
            descriptor_bytes_property_name(idx),
            Value::Bytes(Arc::from(vec![idx as u8; 16])),
        )
    });
    PropertyMap::from_pairs(
        [(property_name(0), Value::Int(row as i64))]
            .into_iter()
            .chain(string_values)
            .chain(byte_values),
    )
    .expect("descriptor properties fit core caps")
}

fn descriptor_update_diff(row: usize) -> PropertyDiff {
    let string_values = (0..4).map(|idx| {
        (
            descriptor_string_property_name(idx),
            Value::String(label(&format!("updated-{row}-text-{idx}"))),
        )
    });
    let byte_values = (0..4).map(|idx| {
        (
            descriptor_bytes_property_name(idx),
            Value::Bytes(Arc::from(vec![(idx + 1) as u8; 16])),
        )
    });
    PropertyDiff::new(string_values.chain(byte_values), []).expect("property diff is valid")
}

fn property_name(idx: usize) -> DbString {
    match idx {
        0 => label("score"),
        1 => label("name"),
        2 => label("bench_id"),
        _ => label(&format!("extra_{idx}")),
    }
}

fn descriptor_string_property_name(idx: usize) -> DbString {
    label(&format!("text_{idx}"))
}

fn descriptor_bytes_property_name(idx: usize) -> DbString {
    label(&format!("bytes_{idx}"))
}

fn label(value: &str) -> DbString {
    db_string(value).expect("bench string fits DB string cap")
}

criterion_group! {
    name = bound_type_validation;
    config = common::criterion_config();
    targets = bench_bound_type_validation
}
criterion_main!(bound_type_validation);
