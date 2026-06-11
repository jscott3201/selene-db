use super::*;

fn vector(values: &[f32]) -> VectorValue {
    VectorValue::new(values.to_vec()).unwrap()
}

#[test]
fn turbo_quant_search_exact_reranks_compressed_candidates() {
    let mut index = TurboQuantVectorIndex::new(3).unwrap();
    index.insert(10, vector(&[1.0, 0.0, 0.0])).unwrap();
    index.insert(2, vector(&[0.9, 0.1, 0.0])).unwrap();
    index.insert(7, vector(&[0.0, 1.0, 0.0])).unwrap();
    index.finish_bulk_load().unwrap();

    let hits = index.search(&vector(&[1.0, 0.0, 0.0]), 2, 3).unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.row).collect::<Vec<_>>(),
        vec![10, 2]
    );
    assert_eq!(hits[0].distance, 0.0);
    assert!(hits[1].distance > hits[0].distance);
}

#[test]
fn turbo_quant_update_delete_and_memory_usage_track_stale_entries() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, vector(&[1.0, 0.0])).unwrap();
    index.insert(2, vector(&[0.0, 1.0])).unwrap();
    index.finish_bulk_load().unwrap();

    index.remove(1);
    index.insert(2, vector(&[1.0, 0.0])).unwrap();

    let hits = index.search(&vector(&[1.0, 0.0]), 5, 5).unwrap();
    assert_eq!(hits.iter().map(|hit| hit.row).collect::<Vec<_>>(), vec![2]);

    let usage = index.memory_usage();
    assert_eq!(usage.entries, 3);
    assert_eq!(usage.live_entries, 1);
    assert_eq!(usage.deleted_entries, 2);
    assert!(usage.code_bytes > 0);
    assert!(usage.codebook_bytes > 0);
    assert!(usage.calibration_bytes > 0);
    assert!(usage.estimated_heap_bytes >= usage.code_bytes);
    assert!(usage.referenced_vector_bytes >= 3 * 2 * size_of::<f32>());
}
