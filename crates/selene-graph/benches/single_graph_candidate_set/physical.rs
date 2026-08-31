use criterion::{BenchmarkId, Criterion, Throughput};
use roaring::RoaringBitmap;
use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, db_string};
use selene_graph::{CandidateSet, Node, RowIndex, SeleneGraph, SharedGraph, TypedIndexKind};

const DENSITIES: &[(usize, usize)] = &[(1_000, 10), (1_000, 100), (1_000, 500)];

pub(super) fn bench_physical_candidate_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_physical_candidate_set");
    for &(scale, width) in DENSITIES {
        let fixture = PhysicalCandidateFixture::build(scale, width);
        group.throughput(Throughput::Elements(width as u64));
        for (name, operation) in [
            ("union", Algebra::Union),
            ("intersection", Algebra::Intersection),
            ("difference", Algebra::Difference),
        ] {
            group.bench_with_input(
                BenchmarkId::new(format!("raw_{name}_d{}", fixture.density), scale),
                &fixture,
                |b, fixture| {
                    b.iter(|| {
                        let rows = operation.raw(&fixture.raw_left, &fixture.raw_right);
                        std::hint::black_box(rows.len());
                    });
                },
            );
            group.bench_with_input(
                BenchmarkId::new(format!("typed_{name}_d{}", fixture.density), scale),
                &fixture,
                |b, fixture| {
                    b.iter(|| {
                        let candidates = operation.typed(&fixture.left, &fixture.right);
                        std::hint::black_box(candidates.len());
                    });
                },
            );
        }
        group.bench_with_input(
            BenchmarkId::new(format!("typed_stable_id_iter_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let count = fixture
                        .left
                        .iter_ids(&fixture.graph)
                        .expect("bench candidates match graph")
                        .map(std::hint::black_box)
                        .count();
                    std::hint::black_box(count);
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(format!("raw_stable_id_iter_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let count = fixture
                        .raw_left
                        .iter()
                        .filter_map(|row| fixture.graph.node_id_for_row(RowIndex::new(row)))
                        .map(std::hint::black_box)
                        .count();
                    std::hint::black_box(count);
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(format!("typed_contains_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    std::hint::black_box(
                        fixture
                            .left
                            .contains_id(&fixture.graph, fixture.probe)
                            .expect("bench candidates match graph"),
                    );
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(format!("raw_contains_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                let row = fixture
                    .graph
                    .row_for_node_id(fixture.probe)
                    .expect("probe is mapped")
                    .get();
                b.iter(|| std::hint::black_box(fixture.raw_left.contains(row)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new(format!("typed_label_producer_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    std::hint::black_box(fixture.graph.node_label_candidates(&fixture.left_label))
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(format!("raw_label_producer_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    std::hint::black_box(fixture.graph.nodes_with_label(&fixture.left_label))
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(
                format!("typed_property_producer_d{}", fixture.density),
                scale,
            ),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    std::hint::black_box(fixture.graph.node_property_eq_candidates(
                        &fixture.universe_label,
                        &fixture.bucket_property,
                        &Value::Int(1),
                    ))
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(format!("raw_property_producer_d{}", fixture.density), scale),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    std::hint::black_box(fixture.graph.nodes_with_property_eq(
                        &fixture.universe_label,
                        &fixture.bucket_property,
                        &Value::Int(1),
                    ))
                });
            },
        );
    }
    group.finish();
}

#[derive(Clone, Copy)]
enum Algebra {
    Union,
    Intersection,
    Difference,
}

impl Algebra {
    fn raw(self, left: &RoaringBitmap, right: &RoaringBitmap) -> RoaringBitmap {
        let mut result = left.clone();
        match self {
            Self::Union => result |= right,
            Self::Intersection => result &= right,
            Self::Difference => result -= right,
        }
        result
    }

    fn typed(self, left: &CandidateSet<Node>, right: &CandidateSet<Node>) -> CandidateSet<Node> {
        match self {
            Self::Union => left.union(right),
            Self::Intersection => left.intersection(right),
            Self::Difference => left.difference(right),
        }
        .expect("bench candidates share scope")
    }
}

struct PhysicalCandidateFixture {
    graph: SeleneGraph,
    left: CandidateSet<Node>,
    right: CandidateSet<Node>,
    raw_left: RoaringBitmap,
    raw_right: RoaringBitmap,
    left_label: DbString,
    universe_label: DbString,
    bucket_property: DbString,
    probe: NodeId,
    density: usize,
}

impl PhysicalCandidateFixture {
    fn build(scale: usize, width: usize) -> Self {
        let left_label = db_string("CandidateLeft").unwrap();
        let right_label = db_string("CandidateRight").unwrap();
        let universe_label = db_string("CandidateUniverse").unwrap();
        let bucket_property = db_string("bucket").unwrap();
        let shared = SharedGraph::new(GraphId::new(30_000 + width as u64));
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        let right_start = width / 2;
        let mut probe = NodeId::TOMBSTONE;
        for row in 0..scale {
            let mut labels = LabelSet::single(universe_label.clone());
            if row < width {
                labels.insert(left_label.clone());
            }
            if (right_start..right_start + width).contains(&row) {
                labels.insert(right_label.clone());
            }
            let node = mutator
                .create_node(
                    labels,
                    PropertyMap::from_pairs([(
                        bucket_property.clone(),
                        Value::Int(i64::from(row < width)),
                    )])
                    .unwrap(),
                )
                .unwrap();
            if row + 1 == width {
                probe = node;
            }
        }
        mutator
            .create_property_index(
                universe_label.clone(),
                bucket_property.clone(),
                TypedIndexKind::I64,
            )
            .unwrap();
        txn.commit().unwrap();
        let graph = shared.read().as_ref().clone();
        let left = graph.node_label_candidates(&left_label);
        let right = graph.node_label_candidates(&right_label);
        let raw_left = graph.nodes_with_label(&left_label).unwrap().clone();
        let raw_right = graph.nodes_with_label(&right_label).unwrap().clone();
        Self {
            graph,
            left,
            right,
            raw_left,
            raw_right,
            left_label,
            universe_label,
            bucket_property,
            probe,
            density: width * 100 / scale,
        }
    }
}
