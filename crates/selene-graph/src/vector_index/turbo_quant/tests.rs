use super::*;

fn vector(values: &[f32]) -> VectorValue {
    VectorValue::new(values.to_vec()).unwrap()
}

#[test]
fn turbo_quant_candidates_rank_compressed_rows() {
    let mut index = TurboQuantVectorIndex::new(3).unwrap();
    index.insert(10, &vector(&[1.0, 0.0, 0.0])).unwrap();
    index.insert(2, &vector(&[0.9, 0.1, 0.0])).unwrap();
    index.insert(7, &vector(&[0.0, 1.0, 0.0])).unwrap();
    index.finish_bulk_load().unwrap();

    let hits = index.candidates(&vector(&[1.0, 0.0, 0.0]), 2, 2).unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.row).collect::<Vec<_>>(),
        vec![10, 2]
    );
    assert!(hits[0].distance <= hits[1].distance);
}

#[test]
fn turbo_quant_update_delete_and_memory_usage_track_stale_entries() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, &vector(&[1.0, 0.0])).unwrap();
    index.insert(2, &vector(&[0.0, 1.0])).unwrap();
    index.finish_bulk_load().unwrap();

    index.remove(1);
    index.insert(2, &vector(&[1.0, 0.0])).unwrap();

    let hits = index.candidates(&vector(&[1.0, 0.0]), 5, 5).unwrap();
    assert_eq!(hits.iter().map(|hit| hit.row).collect::<Vec<_>>(), vec![2]);

    let usage = index.memory_usage();
    assert_eq!(usage.entries, 3);
    assert_eq!(usage.live_entries, 1);
    assert_eq!(usage.deleted_entries, 2);
    assert!(usage.code_bytes > 0);
    assert!(usage.codebook_bytes > 0);
    assert!(usage.calibration_bytes > 0);
    assert!(usage.estimated_heap_bytes >= usage.code_bytes);
    assert_eq!(usage.referenced_vector_bytes, 0);
}

#[test]
fn turbo_quant_search_uses_live_map_when_stale_slots_dominate() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    for row in 0..80 {
        index
            .insert(row, &vector(&[1.0 + row as f32 * 0.001, 0.0]))
            .unwrap();
    }
    index.finish_bulk_load().unwrap();
    for row in 0..79 {
        index.remove(row);
    }

    assert!(!index.should_scan_by_slot_order());

    let hits = index.candidates(&vector(&[1.0, 0.0]), 5, 5).unwrap();
    assert_eq!(hits.iter().map(|hit| hit.row).collect::<Vec<_>>(), vec![79]);

    let batch = index
        .candidates_batch(&[vector(&[1.0, 0.0]), vector(&[0.5, 0.5])], 5, 5)
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(
        batch[0].iter().map(|hit| hit.row).collect::<Vec<_>>(),
        vec![79]
    );
    assert_eq!(
        batch[1].iter().map(|hit| hit.row).collect::<Vec<_>>(),
        vec![79]
    );
}

#[test]
fn turbo_quant_parallel_slot_scan_matches_single_thread_hits() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..32 {
        index
            .insert(
                row,
                &vector(&[
                    1.0 + row as f32 * 0.01,
                    (row % 5) as f32 * 0.1,
                    (row % 7) as f32 * 0.05,
                    0.25,
                ]),
            )
            .unwrap();
    }
    index.finish_bulk_load().unwrap();

    let query = vector(&[1.0, 0.2, 0.1, 0.25]);
    let single_thread = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let two_threads = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();

    let sequential = single_thread.install(|| {
        assert!(!index.should_parallelize_slot_scan(8));
        index.candidates(&query, 4, 8).unwrap()
    });
    let parallel = two_threads.install(|| {
        assert!(index.should_parallelize_slot_scan(8));
        index.candidates(&query, 4, 8).unwrap()
    });

    assert_eq!(parallel, sequential);
}

#[test]
fn turbo_quant_batch_candidates_match_single_queries() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..32 {
        index
            .insert(
                row,
                &vector(&[
                    1.0 + row as f32 * 0.01,
                    (row % 5) as f32 * 0.1,
                    (row % 7) as f32 * 0.05,
                    0.25,
                ]),
            )
            .unwrap();
    }
    index.finish_bulk_load().unwrap();

    let queries = [
        vector(&[1.0, 0.2, 0.1, 0.25]),
        vector(&[1.1, 0.0, 0.3, 0.25]),
        vector(&[0.8, 0.4, 0.2, 0.25]),
    ];
    let single_thread = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let two_threads = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();

    let singles = queries
        .iter()
        .map(|query| index.candidates(query, 4, 8).unwrap())
        .collect::<Vec<_>>();
    let sequential = single_thread.install(|| {
        assert!(!index.should_parallelize_slot_scan(8));
        index.candidates_batch(&queries, 4, 8).unwrap()
    });
    let parallel = two_threads.install(|| {
        assert!(index.should_parallelize_slot_scan(8));
        index.candidates_batch(&queries, 4, 8).unwrap()
    });

    assert_eq!(sequential, singles);
    assert_eq!(parallel, singles);
}
