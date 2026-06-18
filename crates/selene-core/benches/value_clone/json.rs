use std::hint::black_box;

use criterion::{Throughput, measurement::WallTime};
use selene_core::JsonValue;

pub(super) fn bench_json_canonical(group: &mut criterion::BenchmarkGroup<'_, WallTime>) {
    let json_metadata = json_metadata_value();
    group.bench_function("json_canonical_string_metadata", |b| {
        b.iter(|| black_box(json_metadata.to_canonical_string()));
    });

    let json_wide = wide_json_object(64);
    group.throughput(Throughput::Elements(64));
    group.bench_function("json_canonical_string_object64", |b| {
        b.iter(|| black_box(json_wide.to_canonical_string()));
    });

    let json_metadata_text = json_metadata_text();
    group.bench_function("json_parse_metadata", |b| {
        b.iter(|| {
            black_box(
                JsonValue::parse_str(black_box(json_metadata_text)).expect("bench JSON parses"),
            )
        });
    });

    let json_wide_text = json_wide.to_canonical_string();
    group.bench_function("json_parse_object64", |b| {
        b.iter(|| {
            black_box(
                JsonValue::parse_str(black_box(json_wide_text.as_str()))
                    .expect("bench JSON parses"),
            )
        });
    });
}

fn json_metadata_value() -> JsonValue {
    JsonValue::parse_str(json_metadata_text()).expect("bench JSON parses")
}

fn json_metadata_text() -> &'static str {
    r#"{
            "z_source": "planner",
            "memory": {
                "kind": "episodic",
                "score": 91,
                "facts": [
                    {"title": "alpha", "current": true},
                    {"title": "beta", "current": false},
                    {"title": "gamma", "current": true}
                ]
            },
            "a_scope": ["graph", "vector", "json"],
            "updated_at": "2026-06-17T12:00:00Z"
        }"#
}

fn wide_json_object(width: usize) -> JsonValue {
    let mut map = serde_json::Map::new();
    for idx in (0..width).rev() {
        map.insert(
            format!("field_{idx:04}"),
            serde_json::json!({
                "rank": idx,
                "label": format!("candidate_{idx:04}"),
                "active": idx % 2 == 0
            }),
        );
    }
    JsonValue::new(serde_json::Value::Object(map)).expect("bench JSON is valid")
}
