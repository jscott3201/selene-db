use super::*;

pub(super) fn bench_exact_json_contains_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_json_contains_scan");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = JsonFixture::build(scale);
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::new("nested_metadata_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_contains_nodes(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.candidate(),
                            10,
                        )
                        .expect("JSON containment scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("nested_metadata_k10_checked_with_deadline", fixture.scale()),
            &fixture,
            |b, fixture| {
                let checker = deadline_checker();
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_contains_nodes_checked(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.candidate(),
                            10,
                            checker,
                        )
                        .expect("JSON containment scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

pub(super) fn bench_exact_json_path_exists_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_json_path_exists_scan");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = JsonFixture::build(scale);
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::new("nested_score_path_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_exists_nodes(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.path(),
                            10,
                        )
                        .expect("JSON path-existence scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(
                "nested_score_path_k10_checked_with_deadline",
                fixture.scale(),
            ),
            &fixture,
            |b, fixture| {
                let checker = deadline_checker();
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_exists_nodes_checked(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.path(),
                            10,
                            checker,
                        )
                        .expect("JSON path-existence scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("nested_score_path_candidates_sorted_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_exists_candidate_nodes(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.path(),
                            fixture.sorted_candidates(),
                            10,
                        )
                        .expect("JSON path-existence candidate scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("nested_score_path_candidates_reverse_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_exists_candidate_nodes(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.path(),
                            fixture.reverse_candidates(),
                            10,
                        )
                        .expect("JSON path-existence candidate scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

pub(super) fn bench_exact_json_path_contains_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_json_path_contains_scan");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = JsonFixture::build(scale);
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::new("nested_memory_path_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_contains_nodes(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.contains_path(),
                            fixture.path_candidate(),
                            10,
                        )
                        .expect("JSON path-containment scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(
                "nested_memory_path_k10_checked_with_deadline",
                fixture.scale(),
            ),
            &fixture,
            |b, fixture| {
                let checker = deadline_checker();
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_contains_nodes_checked(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.contains_path(),
                            fixture.path_candidate(),
                            10,
                            checker,
                        )
                        .expect("JSON path-containment scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

pub(super) fn bench_exact_json_path_value_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_json_path_value_scan");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = JsonFixture::build(scale);
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::new("nested_score_path_k10", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_value_nodes(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.path(),
                            10,
                        )
                        .expect("JSON path-value scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new(
                "nested_score_path_k10_checked_with_deadline",
                fixture.scale(),
            ),
            &fixture,
            |b, fixture| {
                let checker = deadline_checker();
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .exact_json_path_value_nodes_checked(
                            fixture.label(),
                            fixture.payload_key(),
                            fixture.path(),
                            10,
                            checker,
                        )
                        .expect("JSON path-value scan succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

#[derive(Clone, Debug)]
struct JsonFixture {
    scale: usize,
    graph: SeleneGraph,
    label: DbString,
    payload_key: DbString,
    candidate: JsonValue,
    path: Vec<JsonPathSelector>,
    contains_path: Vec<JsonPathSelector>,
    path_candidate: JsonValue,
    sorted_candidates: Vec<NodeId>,
    reverse_candidates: Vec<NodeId>,
}

impl JsonFixture {
    fn build(scale: usize) -> Self {
        let scale = scale.max(1);
        let label = db_string("JsonDoc").expect("bench label is valid");
        let payload_key = db_string("payload").expect("bench key is valid");
        let candidate =
            JsonValue::new(serde_json::json!({"memory": {"kind": "episodic"}, "state": "current"}))
                .expect("bench JSON candidate is valid");
        let path = vec![
            JsonPathSelector::Key(db_string("memory").expect("bench path key is valid")),
            JsonPathSelector::Key(db_string("score").expect("bench path key is valid")),
        ];
        let contains_path = vec![JsonPathSelector::Key(
            db_string("memory").expect("bench path key is valid"),
        )];
        let path_candidate = JsonValue::new(serde_json::json!({"kind": "episodic"}))
            .expect("bench JSON path candidate is valid");
        let shared = SharedGraph::new(GraphId::new(9_500 + scale as u64));
        let mut sorted_candidates = Vec::with_capacity(scale);
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let value = match idx % 4 {
                    0 => json_value(serde_json::json!({
                        "memory": {"kind": "episodic", "score": idx},
                        "state": "current",
                        "tags": ["agent", "graph", "json"]
                    })),
                    1 => json_value(serde_json::json!({
                        "memory": {"kind": "semantic", "score": idx},
                        "state": "current"
                    })),
                    2 => json_value(serde_json::json!({
                        "memory": {"kind": "episodic", "score": idx},
                        "state": "stale"
                    })),
                    _ => Value::String(db_string("not-json").expect("bench string is valid")),
                };
                let props = PropertyMap::from_pairs([(payload_key.clone(), value)])
                    .expect("bench JSON properties are valid");
                let node = mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench JSON node insert succeeds");
                sorted_candidates.push(node);
            }
            txn.commit().expect("bench JSON fixture commit succeeds");
        }
        let mut reverse_candidates = sorted_candidates.clone();
        reverse_candidates.reverse();
        Self {
            scale,
            graph: shared.read().as_ref().clone(),
            label,
            payload_key,
            candidate,
            path,
            contains_path,
            path_candidate,
            sorted_candidates,
            reverse_candidates,
        }
    }

    const fn graph(&self) -> &SeleneGraph {
        &self.graph
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    const fn label(&self) -> &DbString {
        &self.label
    }

    const fn payload_key(&self) -> &DbString {
        &self.payload_key
    }

    const fn candidate(&self) -> &JsonValue {
        &self.candidate
    }

    fn path(&self) -> &[JsonPathSelector] {
        &self.path
    }

    fn contains_path(&self) -> &[JsonPathSelector] {
        &self.contains_path
    }

    const fn path_candidate(&self) -> &JsonValue {
        &self.path_candidate
    }

    fn sorted_candidates(&self) -> &[NodeId] {
        &self.sorted_candidates
    }

    fn reverse_candidates(&self) -> &[NodeId] {
        &self.reverse_candidates
    }
}

fn json_value(value: serde_json::Value) -> Value {
    Value::Json(JsonValue::new(value).expect("bench JSON is valid"))
}
