use super::*;

#[cfg(not(test))]
const TURBO_QUANT_BATCH_PARALLEL_CHUNK_ENTRIES: usize = 256;
#[cfg(test)]
const TURBO_QUANT_BATCH_PARALLEL_CHUNK_ENTRIES: usize = 4;

struct PreparedTurboQuantQuery {
    byte_lut: Vec<f64>,
    query_bias: f64,
}

impl TurboQuantVectorIndex {
    pub(crate) fn candidates_batch(
        &self,
        queries: &[VectorValue],
        k: usize,
        search_width: usize,
    ) -> CoreResult<Vec<Vec<TurboQuantVectorHit>>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if k == 0 || self.live_entries == 0 {
            return Ok(vec![Vec::new(); queries.len()]);
        }
        if !self.should_fuse_batch_scan(queries.len()) {
            return queries
                .iter()
                .map(|query| self.candidates(query, k, search_width))
                .collect();
        }
        let candidate_limit = search_width.max(k).min(self.live_entries);
        let candidates = if self.should_scan_by_slot_order() {
            if let Some(prepared) = self.prepare_fast_scan_queries(queries) {
                self.slot_order_candidates_fast_scan_batch(&prepared, candidate_limit)
            } else {
                let prepared = queries
                    .iter()
                    .map(|query| self.prepare_query(query))
                    .collect::<Vec<_>>();
                self.slot_order_candidates_batch(&prepared, candidate_limit)
            }
        } else {
            let prepared = queries
                .iter()
                .map(|query| self.prepare_query(query))
                .collect::<Vec<_>>();
            self.live_map_candidates_batch(&prepared, candidate_limit)
        };

        Ok(candidates
            .into_iter()
            .map(|query_candidates| {
                query_candidates
                    .into_hits()
                    .into_iter()
                    .map(|hit| TurboQuantVectorHit {
                        row: hit.key.1,
                        distance: hit.distance,
                    })
                    .collect()
            })
            .collect())
    }

    pub(crate) fn should_fuse_batch_scan(&self, query_count: usize) -> bool {
        query_count > 1 && self.supports_fast_scan_accumulator()
    }

    fn prepare_query(&self, query: &VectorValue) -> PreparedTurboQuantQuery {
        let rotated_query = rotated_unit_vector(query, self.dimension);
        PreparedTurboQuantQuery {
            query_bias: query_bias(&rotated_query, &self.shift),
            byte_lut: self.byte_lut(&rotated_query),
        }
    }

    fn slot_order_candidates_batch(
        &self,
        queries: &[PreparedTurboQuantQuery],
        candidate_limit: usize,
    ) -> Vec<VectorTopK<(usize, u32)>> {
        if self.should_parallelize_slot_scan(candidate_limit) {
            return self.slot_order_candidates_batch_parallel(queries, candidate_limit);
        }
        self.slot_order_candidates_batch_blocks(
            0,
            self.codes.block_count(),
            queries,
            candidate_limit,
        )
    }

    fn slot_order_candidates_batch_parallel(
        &self,
        queries: &[PreparedTurboQuantQuery],
        candidate_limit: usize,
    ) -> Vec<VectorTopK<(usize, u32)>> {
        let chunk_blocks =
            TURBO_QUANT_BATCH_PARALLEL_CHUNK_ENTRIES.div_ceil(TURBO_QUANT_BLOCK_ROWS);
        (0..self.codes.block_count())
            .into_par_iter()
            .chunks(chunk_blocks.max(1))
            .map(|blocks| {
                let start = blocks.first().copied().unwrap_or_default();
                let end = blocks.last().copied().map_or(start, |block| block + 1);
                self.slot_order_candidates_batch_blocks(start, end, queries, candidate_limit)
            })
            .reduce(
                || candidate_top_k_batch(queries.len(), candidate_limit),
                merge_candidate_top_k_batch,
            )
    }

    fn slot_order_candidates_batch_blocks(
        &self,
        start_block: usize,
        end_block: usize,
        queries: &[PreparedTurboQuantQuery],
        candidate_limit: usize,
    ) -> Vec<VectorTopK<(usize, u32)>> {
        let mut candidates = candidate_top_k_batch(queries.len(), candidate_limit);
        let mut dots = vec![[0.0; TURBO_QUANT_BLOCK_ROWS]; queries.len()];
        for block in start_block..end_block {
            let block_len = self.codes.block_len(block);
            for (query_dots, query) in dots.iter_mut().zip(queries) {
                query_dots[..block_len].fill(query.query_bias);
            }
            for byte in 0..self.bytes_per_row {
                let lut_base = byte * 256;
                let codes = self.codes.block_byte(block, byte);
                for (query_dots, query) in dots.iter_mut().zip(queries) {
                    for lane in 0..block_len {
                        query_dots[lane] += query.byte_lut[lut_base + usize::from(codes[lane])];
                    }
                }
            }
            let base_slot = block * TURBO_QUANT_BLOCK_ROWS;
            for lane in 0..block_len {
                let slot = base_slot + lane;
                let entry = &self.entries[slot];
                if entry.deleted {
                    continue;
                }
                debug_assert_eq!(self.row_to_entry.get(&entry.row), Some(&slot));
                push_batch_block_distances(&mut candidates, &dots, lane, slot, entry.row, self);
            }
        }
        candidates
    }

    fn live_map_candidates_batch(
        &self,
        queries: &[PreparedTurboQuantQuery],
        candidate_limit: usize,
    ) -> Vec<VectorTopK<(usize, u32)>> {
        let mut candidates = candidate_top_k_batch(queries.len(), candidate_limit);
        let mut distances = vec![0.0; queries.len()];
        for (&row, &slot) in &self.row_to_entry {
            let Some(entry) = self.entries.get(slot) else {
                continue;
            };
            if entry.deleted || entry.row != row {
                continue;
            }
            self.approx_distances_lut_batch(slot, queries, &mut distances);
            push_batch_distances(&mut candidates, &distances, slot, row);
        }
        candidates
    }

    fn approx_distances_lut_batch(
        &self,
        slot: usize,
        queries: &[PreparedTurboQuantQuery],
        distances: &mut [f64],
    ) {
        debug_assert_eq!(queries.len(), distances.len());
        for (distance, query) in distances.iter_mut().zip(queries) {
            *distance = query.query_bias;
        }

        for byte in 0..self.bytes_per_row {
            let lut_offset = byte * 256
                + usize::from(
                    self.codes
                        .row_byte(slot, byte)
                        .expect("TurboQuant slot byte is in bounds"),
                );
            for (distance, query) in distances.iter_mut().zip(queries) {
                *distance += query.byte_lut[lut_offset];
            }
        }

        let scale = f64::from(self.row_scales[slot]);
        for distance in distances {
            *distance = -(*distance * scale);
        }
    }
}

fn candidate_top_k_batch(
    query_count: usize,
    candidate_limit: usize,
) -> Vec<VectorTopK<(usize, u32)>> {
    (0..query_count)
        .map(|_| VectorTopK::new(candidate_limit))
        .collect()
}

fn merge_candidate_top_k_batch(
    mut lhs: Vec<VectorTopK<(usize, u32)>>,
    rhs: Vec<VectorTopK<(usize, u32)>>,
) -> Vec<VectorTopK<(usize, u32)>> {
    for (lhs_query, rhs_query) in lhs.iter_mut().zip(rhs) {
        for hit in rhs_query.into_hits() {
            lhs_query.push_distance(hit.key, hit.distance);
        }
    }
    lhs
}

fn push_batch_distances(
    candidates: &mut [VectorTopK<(usize, u32)>],
    distances: &[f64],
    slot: usize,
    row: u32,
) {
    for (candidate, distance) in candidates.iter_mut().zip(distances.iter().copied()) {
        candidate.push_distance((slot, row), distance);
    }
}

fn push_batch_block_distances(
    candidates: &mut [VectorTopK<(usize, u32)>],
    dots: &[[f64; TURBO_QUANT_BLOCK_ROWS]],
    lane: usize,
    slot: usize,
    row: u32,
    index: &TurboQuantVectorIndex,
) {
    let scale = f64::from(index.row_scales[slot]);
    for (candidate, query_dots) in candidates.iter_mut().zip(dots) {
        candidate.push_distance((slot, row), -(query_dots[lane] * scale));
    }
}
