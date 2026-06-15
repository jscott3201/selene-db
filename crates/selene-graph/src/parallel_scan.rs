//! Shared Rayon chunking helpers for cancellation-aware scan reductions.

use rayon::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{CancellationCause, CancellationChecker};

/// Return true when a scan has enough work to amortize Rayon fan-out.
pub(crate) fn should_parallelize_scan(item_count: u64, k: usize, min_items: u64) -> bool {
    k != 0 && item_count >= min_items && rayon::current_num_threads() > 1
}

/// Collect bitmap rows and run a cancellation-aware parallel chunk reduction.
pub(crate) fn try_reduce_bitmap_chunks<T, E, Identity, Map, Reduce>(
    rows: &RoaringBitmap,
    chunk_len: usize,
    checker: CancellationChecker<'_>,
    identity: Identity,
    map: Map,
    reduce: Reduce,
) -> Result<T, E>
where
    T: Send,
    E: From<CancellationCause> + Send,
    Identity: Fn() -> T + Sync + Send,
    Map: Fn(&[u32]) -> Result<T, E> + Sync + Send,
    Reduce: Fn(T, T) -> Result<T, E> + Sync + Send,
{
    let raw_rows: Vec<u32> = rows.iter().collect();
    try_reduce_chunks(&raw_rows, chunk_len, checker, identity, map, reduce)
}

/// Run a cancellation-aware parallel chunk reduction over a slice.
pub(crate) fn try_reduce_chunks<Item, T, E, Identity, Map, Reduce>(
    items: &[Item],
    chunk_len: usize,
    checker: CancellationChecker<'_>,
    identity: Identity,
    map: Map,
    reduce: Reduce,
) -> Result<T, E>
where
    Item: Sync,
    T: Send,
    E: From<CancellationCause> + Send,
    Identity: Fn() -> T + Sync + Send,
    Map: Fn(&[Item]) -> Result<T, E> + Sync + Send,
    Reduce: Fn(T, T) -> Result<T, E> + Sync + Send,
{
    debug_assert!(chunk_len > 0, "parallel scan chunk length must be nonzero");
    items
        .par_chunks(chunk_len)
        .map(|chunk| {
            checker.note_nodes_scanned(chunk.len()).map_err(E::from)?;
            map(chunk)
        })
        .try_reduce(identity, reduce)
}

#[cfg(test)]
mod tests {
    use selene_core::{CancellationCause, CancellationToken};

    use super::*;

    #[test]
    fn disabled_checker_reduces_chunks() {
        let items = [1_u32, 2, 3, 4, 5, 6];
        let result: Result<u32, CancellationCause> = try_reduce_chunks(
            &items,
            2,
            CancellationChecker::disabled(),
            || 0_u32,
            |chunk| Ok(chunk.iter().sum::<u32>()),
            |lhs, rhs| Ok(lhs + rhs),
        );
        let sum = result.expect("disabled scan succeeds");

        assert_eq!(sum, 21);
    }

    #[test]
    fn cancelled_checker_stops_before_chunk_body() {
        let token = CancellationToken::new();
        token.cancel();
        let items = [1_u32, 2, 3, 4];
        let result: Result<u32, CancellationCause> = try_reduce_chunks(
            &items,
            2,
            CancellationChecker::new(Some(&token), None),
            || 0_u32,
            |_chunk| {
                panic!("chunk body must not run once cancellation is observed");
            },
            |lhs, rhs| Ok(lhs + rhs),
        );

        assert_eq!(result, Err(CancellationCause::Cancelled));
    }
}
