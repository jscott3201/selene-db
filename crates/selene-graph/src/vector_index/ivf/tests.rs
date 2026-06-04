use std::borrow::Cow;

use selene_core::VectorMetric;

use super::*;

fn vector(values: &[f32]) -> VectorValue {
    VectorValue::new(values.to_vec()).expect("test vector is valid")
}

#[test]
fn ivf_target_centroid_count_scales_until_cap() {
    assert_eq!(target_centroid_count(0), 1);
    assert_eq!(target_centroid_count(100_000), 317);
    assert_eq!(target_centroid_count(10_000_000), MAX_CENTROIDS);
}

#[test]
fn ivf_training_entry_ids_borrow_when_below_sample_cap() {
    let live_entries = (0..100_000).collect::<Vec<_>>();

    let training_entries = training_entry_ids(&live_entries);

    assert!(matches!(training_entries, Cow::Borrowed(_)));
    assert_eq!(training_entries.as_ref(), live_entries.as_slice());
}

#[test]
fn ivf_training_entry_ids_sample_evenly_when_above_cap() {
    let live_entries = (0..250_000).collect::<Vec<_>>();

    let training_entries = training_entry_ids(&live_entries);

    assert!(matches!(training_entries, Cow::Owned(_)));
    assert_eq!(training_entries.len(), TRAINING_SAMPLE_MAX_ENTRIES);
    assert_eq!(training_entries[0], 0);
    assert_eq!(training_entries[training_entries.len() - 1], 249_999);
    assert!(training_entries.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn ivf_search_finds_near_rows_when_all_lists_are_probed() {
    let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
    for row in 0..32 {
        index.insert(row, vector(&[row as f32, 0.0])).unwrap();
    }
    index.finish_bulk_load().unwrap();

    let usage = index.memory_usage();
    assert!(!index.has_stale_entries());
    let hits = index
        .search(&vector(&[4.1, 0.0]), 3, usage.list_count)
        .unwrap();

    assert_eq!(hits[0].row, 4);
    assert!(hits.iter().any(|hit| hit.row == 5));
    assert_eq!(usage.live_entries, 32);
    assert_eq!(usage.assigned_entries, 32);
    assert!(usage.non_empty_list_count > 0);
    assert!(usage.max_list_len > 0);
    assert_eq!(
        usage.average_list_len_basis_points,
        usage.assigned_entries * 10_000 / usage.list_count
    );
    assert!(usage.centroids > 1);
}

#[test]
fn ivf_memory_usage_reports_list_distribution() {
    let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
    for row in 0..16 {
        index.insert(row, vector(&[row as f32, 0.0])).unwrap();
    }

    index.finish_bulk_load().unwrap();

    let usage = index.memory_usage();
    assert_eq!(
        usage.non_empty_list_count,
        index.lists.iter().filter(|list| !list.is_empty()).count()
    );
    assert_eq!(
        usage.max_list_len,
        index.lists.iter().map(Vec::len).max().unwrap()
    );
    assert_eq!(
        usage.average_list_len_basis_points,
        usage.assigned_entries * 10_000 / usage.list_count
    );
}

#[test]
fn ivf_replace_marks_old_row_version_stale() {
    let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
    index.insert(1, vector(&[100.0, 0.0])).unwrap();
    index.insert(2, vector(&[2.0, 0.0])).unwrap();
    index.finish_bulk_load().unwrap();

    index.insert(1, vector(&[1.0, 0.0])).unwrap();

    assert!(index.has_stale_entries());
    let hits = index.search(&vector(&[1.1, 0.0]), 2, 16).unwrap();
    assert_eq!(hits[0].row, 1);
    let usage = index.memory_usage();
    assert_eq!(usage.live_entries, 2);
    assert_eq!(usage.deleted_entries, 1);
}

#[test]
fn ivf_remove_excludes_row_from_results() {
    let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
    index.insert(1, vector(&[1.0, 0.0])).unwrap();
    index.insert(2, vector(&[2.0, 0.0])).unwrap();
    index.finish_bulk_load().unwrap();

    index.remove(1);

    assert!(index.has_stale_entries());
    let hits = index.search(&vector(&[1.0, 0.0]), 2, 16).unwrap();
    assert_eq!(hits[0].row, 2);
    let usage = index.memory_usage();
    assert_eq!(usage.live_entries, 1);
    assert_eq!(usage.deleted_entries, 1);
}

#[test]
fn ivf_finish_bulk_load_rebuilds_lists_after_updates() {
    let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
    for row in 0..12 {
        index.insert(row, vector(&[row as f32, 0.0])).unwrap();
    }
    index.finish_bulk_load().unwrap();
    index.remove(0);
    index.insert(12, vector(&[0.1, 0.0])).unwrap();

    index.finish_bulk_load().unwrap();

    assert!(index.has_stale_entries());
    let hits = index.search(&vector(&[0.0, 0.0]), 1, 16).unwrap();
    assert_eq!(hits[0].row, 12);
    let usage = index.memory_usage();
    assert_eq!(usage.live_entries, 12);
    assert_eq!(usage.assigned_entries, 12);
}

#[test]
fn ivf_parallel_assignment_path_keeps_exact_full_probe_results() {
    let mut index = IvfVectorIndex::new(VectorMetric::SquaredEuclidean);
    for row in 0..16 {
        index.insert(row, vector(&[row as f32, 0.0])).unwrap();
    }

    index.finish_bulk_load().unwrap();

    let usage = index.memory_usage();
    assert!(should_parallelize_assignments(
        usage.live_entries,
        usage.centroids
    ));
    assert!(index.entry_squared_norms.is_empty());
    assert!(index.centroid_squared_norms.is_empty());
    assert_eq!(usage.assigned_entries, 16);
    assert_eq!(index.lists.iter().map(Vec::capacity).sum::<usize>(), 16);

    let hits = index
        .search(&vector(&[9.2, 0.0]), 3, usage.list_count)
        .unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.row).collect::<Vec<_>>(),
        [9, 10, 8]
    );
}

#[test]
fn ivf_cosine_bulk_load_refreshes_centroid_norm_cache() {
    let mut index = IvfVectorIndex::new(VectorMetric::Cosine);
    for row in 0..16 {
        index.insert(row, vector(&[1.0, row as f32 + 1.0])).unwrap();
    }

    index.finish_bulk_load().unwrap();

    assert_eq!(index.entry_squared_norms.len(), index.entries.len());
    assert!(index.entry_squared_norms.iter().all(|norm| *norm > 0.0));
    assert_eq!(index.centroid_squared_norms.len(), index.centroids.len());
    assert!(index.centroid_squared_norms.iter().all(|norm| *norm > 0.0));
    let hits = index
        .search(&vector(&[1.0, 9.1]), 1, index.lists.len())
        .unwrap();
    assert_eq!(hits[0].row, 8);
}

#[test]
fn ivf_cosine_replace_keeps_entry_norm_cache_aligned() {
    let mut index = IvfVectorIndex::new(VectorMetric::Cosine);
    index.insert(1, vector(&[1.0, 0.0])).unwrap();
    index.insert(2, vector(&[0.0, 1.0])).unwrap();
    index.finish_bulk_load().unwrap();

    index.insert(1, vector(&[0.9, 0.1])).unwrap();

    assert_eq!(index.entry_squared_norms.len(), index.entries.len());
    assert_eq!(index.memory_usage().deleted_entries, 1);
    let hits = index.search(&vector(&[1.0, 0.0]), 1, 16).unwrap();
    assert_eq!(hits[0].row, 1);
}

#[test]
fn ivf_cosine_rejects_zero_norm_query() {
    let mut index = IvfVectorIndex::new(VectorMetric::Cosine);
    index.insert(1, vector(&[1.0, 0.0])).unwrap();
    index.finish_bulk_load().unwrap();

    let err = index.search(&vector(&[0.0, 0.0]), 1, 16).unwrap_err();

    assert!(err.to_string().contains("zero-norm vector"));
}
