use selene_core::VectorMetric;

use super::*;

fn vector(value: f32) -> VectorValue {
    VectorValue::new(vec![value, 0.0]).expect("test vector is valid")
}

fn level_zero_links(layers: impl IntoIterator<Item = Vec<u32>>) -> LevelZeroLinks {
    let mut links = LevelZeroLinks::new();
    for layer in layers {
        links.push_for_test(layer);
    }
    links
}

#[test]
fn hnsw_search_finds_near_rows() {
    let mut index =
        HnswVectorIndex::with_config(VectorMetric::SquaredEuclidean, HnswIndexConfig::default());
    for row in 0..64 {
        index.insert(row, vector(row as f32)).unwrap();
    }

    let hits = index.search(&vector(4.1), 3, 32).unwrap();
    assert_eq!(hits[0].row, 4);
    assert!(hits.iter().any(|hit| hit.row == 5));
}

#[test]
fn hnsw_insert_with_scratch_retains_reusable_buffers() {
    let mut index =
        HnswVectorIndex::with_config(VectorMetric::SquaredEuclidean, HnswIndexConfig::default());
    let mut scratch = HnswSearchScratch::default();
    for row in 0..64 {
        index
            .insert_with_scratch(row, vector(row as f32), &mut scratch)
            .unwrap();
    }

    assert!(scratch.visited.capacity() > 0);
    assert!(scratch.candidates.capacity() > 0);
    assert!(scratch.best.capacity() > 0);
    assert!(scratch.result.capacity() > 0);
    assert!(scratch.prune_candidates.capacity() > 0);
}

#[test]
fn hnsw_replace_marks_old_row_version_stale() {
    let mut index =
        HnswVectorIndex::with_config(VectorMetric::SquaredEuclidean, HnswIndexConfig::default());
    index.insert(1, vector(100.0)).unwrap();
    index.insert(2, vector(2.0)).unwrap();
    index.insert(1, vector(1.0)).unwrap();

    let hits = index.search(&vector(1.1), 2, 16).unwrap();
    assert_eq!(hits[0].row, 1);
    assert_eq!(index.live_len(), 2);
}

#[test]
fn hnsw_remove_excludes_row_from_results() {
    let mut index =
        HnswVectorIndex::with_config(VectorMetric::SquaredEuclidean, HnswIndexConfig::default());
    index.insert(1, vector(1.0)).unwrap();
    index.insert(2, vector(2.0)).unwrap();
    index.remove(1);

    let hits = index.search(&vector(1.0), 2, 16).unwrap();
    assert_eq!(hits[0].row, 2);
    assert_eq!(index.live_len(), 1);
}

#[test]
fn hnsw_search_layer_admits_equal_distance_better_id() {
    let index = HnswVectorIndex {
        metric: VectorMetric::SquaredEuclidean,
        nodes: vec![
            HnswNode {
                row: 0,
                vector: vector(0.0),
                deleted: false,
                upper_links: empty_upper_link_layers(0),
            },
            HnswNode {
                row: 1,
                vector: vector(1.0),
                deleted: false,
                upper_links: empty_upper_link_layers(0),
            },
            HnswNode {
                row: 2,
                vector: vector(2.0),
                deleted: false,
                upper_links: empty_upper_link_layers(0),
            },
        ],
        level_zero_links: level_zero_links([vec![2, 1], Vec::new(), Vec::new()]),
        row_to_entry: FxHashMap::default(),
        entry_point: Some(0),
        max_level: 0,
        m: usize::from(HnswIndexConfig::DEFAULT_MAX_NEIGHBORS),
        ef_construction: usize::from(HnswIndexConfig::DEFAULT_EF_CONSTRUCTION),
    };

    let candidates = index
        .search_layer(0, 2, 0, |candidate| {
            Ok(match candidate {
                0 => 0.0,
                1 | 2 => 1.0,
                _ => unreachable!("test graph only has three candidates"),
            })
        })
        .unwrap();

    let ids: Vec<_> = candidates
        .into_iter()
        .map(|candidate| candidate.id)
        .collect();
    assert_eq!(ids, vec![0, 1]);
}

#[test]
fn hnsw_upper_layer_container_is_empty_for_level_zero_nodes() {
    let links = empty_upper_link_layers(0);
    assert_eq!(links.len(), 0);
    assert_eq!(links.capacity(), 0);

    let promoted_links = empty_upper_link_layers(1);
    assert_eq!(promoted_links.len(), 1);
    assert_eq!(promoted_links.capacity(), 1);
}

#[test]
fn hnsw_finish_bulk_load_compacts_level_zero_without_changing_search() {
    let mut index =
        HnswVectorIndex::with_config(VectorMetric::SquaredEuclidean, HnswIndexConfig::default());
    for row in 0..64 {
        index.insert(row, vector(row as f32)).unwrap();
    }
    let before = index.memory_usage();

    index.finish_bulk_load();
    let after = index.memory_usage();

    assert_eq!(after.link_count, before.link_count);
    assert_eq!(after.level_zero_link_count, before.level_zero_link_count);
    assert!(after.estimated_heap_bytes < before.estimated_heap_bytes);
    let hits = index.search(&vector(4.1), 3, 32).unwrap();
    assert_eq!(hits[0].row, 4);
}

#[test]
fn hnsw_compact_level_zero_allows_later_insert_overlay() {
    let mut index =
        HnswVectorIndex::with_config(VectorMetric::SquaredEuclidean, HnswIndexConfig::default());
    for row in 0..64 {
        index.insert(row, vector(row as f32)).unwrap();
    }
    index.finish_bulk_load();

    index.insert(64, vector(64.0)).unwrap();

    let hits = index.search(&vector(64.0), 1, 32).unwrap();
    assert_eq!(hits[0].row, 64);
}
