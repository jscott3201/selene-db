use super::*;
use roaring::RoaringBitmap;

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
fn turbo_quant_update_delete_and_memory_usage_compact_slots() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, &vector(&[1.0, 0.0])).unwrap();
    index.insert(2, &vector(&[0.0, 1.0])).unwrap();
    index.finish_bulk_load().unwrap();

    index.remove(1);
    index.insert(2, &vector(&[1.0, 0.0])).unwrap();

    let hits = index.candidates(&vector(&[1.0, 0.0]), 5, 5).unwrap();
    assert_eq!(hits.iter().map(|hit| hit.row).collect::<Vec<_>>(), vec![2]);

    let usage = index.memory_usage();
    assert_eq!(usage.entries, 1);
    assert_eq!(usage.live_entries, 1);
    assert_eq!(usage.deleted_entries, 0);
    assert!(usage.code_bytes > 0);
    assert!(usage.codebook_bytes > 0);
    assert!(usage.calibration_bytes > 0);
    assert!(usage.estimated_heap_bytes >= usage.code_bytes);
    assert_eq!(usage.referenced_vector_bytes, 0);
}

#[test]
fn turbo_quant_remove_compacts_moved_slots() {
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

    assert!(index.should_scan_by_slot_order());
    assert_eq!(index.slot_for_row(79), Some(0));
    let usage = index.memory_usage();
    assert_eq!(usage.entries, 1);
    assert_eq!(usage.live_entries, 1);
    assert_eq!(usage.deleted_entries, 0);

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
fn turbo_quant_bulk_replacement_preserves_pending_calibration_rows() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, &vector(&[1.0, 0.0])).unwrap();
    index.insert(2, &vector(&[0.0, 1.0])).unwrap();
    index.insert(1, &vector(&[0.9, 0.1])).unwrap();

    assert_eq!(index.slot_for_row(1), Some(0));
    assert_eq!(index.slot_for_row(2), Some(1));
    assert_eq!(index.bulk_rotated.len(), 2 * index.dimension);

    index.finish_bulk_load().unwrap();
    let hits = index.candidates(&vector(&[1.0, 0.0]), 2, 2).unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.row).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn turbo_quant_bulk_remove_preserves_compacted_calibration_rows() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, &vector(&[1.0, 0.0])).unwrap();
    index.insert(2, &vector(&[0.0, 1.0])).unwrap();
    index.insert(3, &vector(&[-1.0, 0.0])).unwrap();

    index.remove(2);

    assert_eq!(index.rows, vec![1, 3]);
    assert_eq!(index.slot_for_row(1), Some(0));
    assert_eq!(index.slot_for_row(3), Some(1));
    assert_eq!(index.bulk_rotated.len(), 2 * index.dimension);

    index.finish_bulk_load().unwrap();
    let hits = index.candidates(&vector(&[-1.0, 0.0]), 2, 2).unwrap();

    assert_eq!(
        hits.iter().map(|hit| hit.row).collect::<Vec<_>>(),
        vec![3, 1]
    );
}

#[test]
fn turbo_quant_finish_bulk_load_rejects_mismatched_bulk_buffer() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, &vector(&[1.0, 0.0])).unwrap();
    index.bulk_rotated.pop();

    let err = index.finish_bulk_load().unwrap_err();

    assert!(matches!(
        err,
        GraphError::Inconsistent { reason }
            if reason.contains("TurboQuant bulk calibration has 1 components")
    ));
    assert!(index.collecting_bulk);
    assert_eq!(index.bulk_rotated.len(), 1);
}

#[test]
fn turbo_quant_replacement_updates_existing_slot_after_calibration() {
    let mut index = TurboQuantVectorIndex::new(2).unwrap();
    index.insert(1, &vector(&[1.0, 0.0])).unwrap();
    index.insert(2, &vector(&[0.0, 1.0])).unwrap();
    index.insert(3, &vector(&[0.5, 0.5])).unwrap();
    index.finish_bulk_load().unwrap();
    let original_slot = index.slot_for_row(2);

    index.insert(2, &vector(&[0.95, 0.05])).unwrap();

    assert_eq!(index.slot_for_row(2), original_slot);
    let usage = index.memory_usage();
    assert_eq!(usage.entries, 3);
    assert_eq!(usage.live_entries, 3);
    assert_eq!(usage.deleted_entries, 0);
    let hits = index.candidates(&vector(&[1.0, 0.0]), 3, 3).unwrap();
    let rows = hits.iter().map(|hit| hit.row).collect::<Vec<_>>();
    assert!(rows.contains(&1));
    assert!(rows.contains(&2));
    assert!(rows.contains(&3));
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
fn turbo_quant_slot_scan_matches_live_map_reference() {
    let mut index = TurboQuantVectorIndex::new(8).unwrap();
    for row in 0..70 {
        index
            .insert(
                row,
                &vector(&[
                    1.0 + row as f32 * 0.001,
                    (row % 3) as f32 * 0.2,
                    (row % 5) as f32 * 0.1,
                    (row % 7) as f32 * 0.05,
                    0.4,
                    0.3,
                    0.2,
                    0.1,
                ]),
            )
            .unwrap();
    }
    index.finish_bulk_load().unwrap();
    index.remove(3);
    index.remove(33);
    index.remove(69);

    let query = vector(&[1.02, 0.2, 0.1, 0.05, 0.4, 0.3, 0.2, 0.1]);
    let rotated_query = rotated_unit_vector(&query, index.dimension);
    let query_bias = query_bias(&rotated_query, &index.shift);
    let byte_lut = index.byte_lut(&rotated_query);

    let slot_order = index
        .slot_order_candidates(&byte_lut, query_bias, 16)
        .into_hits();
    let live_map = index
        .live_map_candidates(&byte_lut, query_bias, 16)
        .into_hits();

    assert_eq!(slot_order, live_map);
}

#[test]
fn turbo_quant_fast_scan_slot_order_matches_lut_reference() {
    let mut index = TurboQuantVectorIndex::new(3).unwrap();
    index.insert(10, &vector(&[1.0, 0.0, 0.0])).unwrap();
    index.insert(11, &vector(&[0.95, 0.1, 0.0])).unwrap();
    index.insert(12, &vector(&[0.7, 0.6, 0.0])).unwrap();
    index.insert(13, &vector(&[0.0, 1.0, 0.0])).unwrap();
    index.insert(14, &vector(&[-1.0, 0.0, 0.0])).unwrap();
    index.finish_bulk_load().unwrap();
    index.remove(14);

    let query = vector(&[1.0, 0.0, 0.0]);
    let rotated_query = rotated_unit_vector(&query, index.dimension);
    let query_bias = query_bias(&rotated_query, &index.shift);
    let byte_lut = index.byte_lut(&rotated_query);

    let fast_scan = index
        .slot_order_candidates_fast_scan(&rotated_query, query_bias, 4)
        .expect("3-dimensional TurboQuant scan supports FastScan")
        .into_hits();
    let reference = index
        .slot_order_candidates(&byte_lut, query_bias, 4)
        .into_hits();

    assert_eq!(
        fast_scan.iter().map(|hit| hit.key).collect::<Vec<_>>(),
        reference.iter().map(|hit| hit.key).collect::<Vec<_>>()
    );
    assert!(fast_scan.iter().all(|hit| hit.distance.is_finite()));
}

#[test]
fn turbo_quant_fast_scan_lut_flushes_oversized_accumulators() {
    let index = TurboQuantVectorIndex::new((u16::MAX as u32 / 2) + 1).unwrap();
    let rotated_query = vec![0.0; index.dimension];

    assert!(index.fast_scan_lut(&rotated_query).is_some());
}

#[test]
fn turbo_quant_candidates_in_rows_match_sparse_live_map_reference() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..80 {
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
    index.remove(47);

    let allowed = [5, 13, 24, 47, 79, 4_000]
        .into_iter()
        .collect::<RoaringBitmap>();
    let query = vector(&[1.0, 0.2, 0.1, 0.25]);
    let rotated_query = rotated_unit_vector(&query, index.dimension);
    let query_bias = query_bias(&rotated_query, &index.shift);
    let byte_lut = index.byte_lut(&rotated_query);

    let filtered = index
        .candidates_in_rows(&query, 4, 8, &allowed)
        .unwrap()
        .into_iter()
        .map(|hit| hit.row)
        .collect::<Vec<_>>();
    let reference = index
        .live_map_candidates_in_rows(&byte_lut, query_bias, 4, &allowed)
        .into_hits()
        .into_iter()
        .map(|hit| hit.key)
        .collect::<Vec<_>>();

    assert_eq!(filtered, reference);
    assert!(!filtered.contains(&47));
    assert!(filtered.iter().all(|row| allowed.contains(*row)));
}

#[test]
fn turbo_quant_filtered_fast_scan_respects_allowed_rows() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..80 {
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
    index.remove(3);
    index.remove(33);

    let allowed = (0..48).collect::<RoaringBitmap>();
    let query = vector(&[1.0, 0.2, 0.1, 0.25]);
    let rotated_query = rotated_unit_vector(&query, index.dimension);
    let query_bias = query_bias(&rotated_query, &index.shift);
    let byte_lut = index.byte_lut(&rotated_query);

    let fast_scan = index
        .slot_order_candidates_fast_scan_in_rows(&rotated_query, query_bias, 16, &allowed)
        .expect("4-dimensional TurboQuant scan supports FastScan")
        .into_hits();
    let reference = index
        .slot_order_candidates_in_rows(&byte_lut, query_bias, 16, &allowed)
        .into_hits();

    assert_eq!(fast_scan.len(), reference.len());
    assert_eq!(fast_scan[0].key, reference[0].key);
    assert!(fast_scan.iter().all(|hit| hit.distance.is_finite()));
    assert!(fast_scan.iter().all(|hit| allowed.contains(hit.key)));
}

#[test]
fn turbo_quant_candidates_in_all_rows_match_unfiltered_candidates() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..70 {
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
    index.remove(9);

    let allowed = (0..70).collect::<RoaringBitmap>();
    let query = vector(&[1.0, 0.2, 0.1, 0.25]);
    let filtered = index.candidates_in_rows(&query, 4, 8, &allowed).unwrap();
    let unfiltered = index.candidates(&query, 4, 8).unwrap();

    assert_eq!(filtered, unfiltered);
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

#[test]
fn turbo_quant_batch_candidates_in_rows_match_single_queries() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..64 {
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
    index.remove(7);

    let queries = [
        vector(&[1.0, 0.2, 0.1, 0.25]),
        vector(&[1.1, 0.0, 0.3, 0.25]),
        vector(&[0.8, 0.4, 0.2, 0.25]),
    ];
    let allowed = [
        (0..48).collect::<RoaringBitmap>(),
        (8..56).collect::<RoaringBitmap>(),
        (16..64).collect::<RoaringBitmap>(),
    ];

    let singles = queries
        .iter()
        .zip(&allowed)
        .map(|(query, allowed)| index.candidates_in_rows(query, 4, 8, allowed).unwrap())
        .collect::<Vec<_>>();
    let batch = index
        .candidates_batch_in_rows(&queries, 4, 8, &allowed)
        .unwrap();

    assert_eq!(batch, singles);

    let shared_allowed = [allowed[0].clone(), allowed[0].clone(), allowed[0].clone()];
    let shared_singles = queries
        .iter()
        .map(|query| index.candidates_in_rows(query, 4, 8, &allowed[0]).unwrap())
        .collect::<Vec<_>>();
    let shared_batch = index
        .candidates_batch_in_rows(&queries, 4, 8, &shared_allowed)
        .unwrap();

    assert_eq!(shared_batch, shared_singles);
}

#[test]
fn turbo_quant_filtered_fast_scan_batch_matches_live_map_reference() {
    let mut index = TurboQuantVectorIndex::new(4).unwrap();
    for row in 0..64 {
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
    index.remove(7);
    index.remove(33);

    let queries = [
        vector(&[1.0, 0.2, 0.1, 0.25]),
        vector(&[1.1, 0.0, 0.3, 0.25]),
        vector(&[0.8, 0.4, 0.2, 0.25]),
    ];
    let allowed = [
        [1, 2, 7, 8, 13, 21, 34, 55]
            .into_iter()
            .collect::<RoaringBitmap>(),
        [3, 5, 11, 17, 23, 29, 33, 47]
            .into_iter()
            .collect::<RoaringBitmap>(),
        [4, 16, 24, 32, 40, 48, 56, 63]
            .into_iter()
            .collect::<RoaringBitmap>(),
    ];
    let candidate_limits = [4, 4, 4];
    let prepared = index
        .prepare_fast_scan_queries(&queries)
        .expect("4-dimensional TurboQuant scan supports FastScan");

    let fast_scan = index
        .slot_order_candidates_fast_scan_batch_in_rows(&prepared, &candidate_limits, &allowed)
        .into_iter()
        .map(|hits| {
            hits.into_hits()
                .into_iter()
                .map(|hit| hit.key)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let reference = queries
        .iter()
        .zip(&allowed)
        .map(|(query, allowed)| {
            let rotated_query = rotated_unit_vector(query, index.dimension);
            let query_bias = query_bias(&rotated_query, &index.shift);
            let byte_lut = index.byte_lut(&rotated_query);
            index
                .live_map_candidates_in_rows(&byte_lut, query_bias, 4, allowed)
                .into_hits()
                .into_iter()
                .map(|hit| hit.key)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    assert_eq!(fast_scan, reference);
    assert!(fast_scan[0].iter().all(|row| *row != 7));
    assert!(fast_scan[1].iter().all(|row| *row != 33));
}

#[test]
fn turbo_quant_high_dimension_batch_candidates_match_single_queries() {
    let dimension = 300;
    let mut index = TurboQuantVectorIndex::new(dimension).unwrap();
    for row in 0..32 {
        index
            .insert(row, &generated_vector(row as usize, dimension as usize))
            .unwrap();
    }
    index.finish_bulk_load().unwrap();

    let queries = [
        generated_vector(7, dimension as usize),
        generated_vector(19, dimension as usize),
        generated_vector(31, dimension as usize),
    ];
    assert!(index.should_fuse_batch_scan(queries.len()));

    let singles = queries
        .iter()
        .map(|query| index.candidates(query, 4, 8).unwrap())
        .collect::<Vec<_>>();
    let batch = index.candidates_batch(&queries, 4, 8).unwrap();

    assert_eq!(batch, singles);
}

fn generated_vector(seed: usize, dimension: usize) -> VectorValue {
    let components = (0..dimension)
        .map(|dim| {
            ((((seed + 3) * (dim + 11)) % 97) as f32 - 48.0) / 48.0
                + (((seed + dim) % 17) as f32 - 8.0) * 0.001
        })
        .collect::<Vec<_>>();
    VectorValue::new(components).unwrap()
}
