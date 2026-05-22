#![allow(missing_docs)]
//! Criterion benches for closed-graph type validation during commit.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    GraphId, GraphTypeId, IStr, LabelDiff, LabelSet, PropertyDiff, PropertyMap, PropertyValueType,
    SchemaChange, Value, intern,
};
use selene_graph::{
    EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef, SeleneGraph, SharedGraph,
    ValidationMode,
};
use selene_testing::BenchProfile;

const UPDATE_BATCH: usize = 100;

fn bench_bound_type_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("bound_type_validation");
    group.throughput(Throughput::Elements(UPDATE_BATCH as u64));
    for &scale in BenchProfile::from_env().scales() {
        let simple = simple_graph_type();
        let rich = rich_graph_type();
        let unbound_graph = graph_snapshot(scale, None, &[label("SimpleNode")], 3);
        let simple_graph = graph_snapshot(scale, Some(simple), &[label("SimpleNode")], 3);
        let rich_labels = (0..10)
            .map(|idx| label(&format!("RichNode{idx}")))
            .collect::<Vec<_>>();
        let rich_graph = graph_snapshot(scale, Some(rich), &rich_labels, 10);

        bench_update_case(&mut group, "unbound_commit", scale, unbound_graph);
        bench_update_case(&mut group, "bound_commit_simple", scale, simple_graph);
        bench_update_case(&mut group, "bound_commit_rich", scale, rich_graph.clone());
        bench_schema_change_case(&mut group, scale, rich_graph);
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
                    mutator.schema_change(
                        GraphId::new(1),
                        SchemaChange::NodeTypeDropped {
                            graph_type: GraphTypeId::new(1).expect("non-zero graph type id"),
                            name: label("NoopDroppedType"),
                        },
                    );
                }
                let changes = txn.commit().expect("schema commit succeeds").changes.len();
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
            let diff = PropertyDiff::new([(score, Value::Int(idx as i64))], [])
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

fn graph_snapshot(
    scale: usize,
    type_def: Option<GraphTypeDef>,
    labels: &[IStr],
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
                let label = labels[idx % labels.len()];
                mutator
                    .create_node(LabelSet::single(label), properties(idx, property_count))
                    .expect("fixture node inserts");
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
            source_node_type: 0,
            target_node_type: 0,
            properties: property_defs(3),
            validation_mode: ValidationMode::Strict,
        }],
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
                source_node_type: idx,
                target_node_type: (idx + 1) % 10,
                properties: property_defs(10),
                validation_mode: ValidationMode::Strict,
            })
            .collect(),
    }
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

fn property_name(idx: usize) -> IStr {
    match idx {
        0 => label("score"),
        1 => label("name"),
        2 => label("bench_id"),
        _ => label(&format!("extra_{idx}")),
    }
}

fn label(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}

criterion_group! {
    name = bound_type_validation;
    config = common::criterion_config();
    targets = bench_bound_type_validation
}
criterion_main!(bound_type_validation);
