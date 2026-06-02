//! Match-mode wrapper operator.
//!
//! Executes the pattern-wide `<match mode>` filter (ISO/IEC 39075:2024 §16.4).
//! Only DIFFERENT EDGES (Feature G002) reaches this operator: it is a post-walk
//! row filter that drops any candidate row binding one graph edge to two
//! positions across the whole graph pattern (§16.4 GR4 / GR8(a)). REPEATABLE
//! ELEMENTS (Feature G003) and the implementation-defined default install no
//! filter at lowering, so they never construct a wrapper here.

use crate::{
    JoinTree, MatchMode, PathContributor,
    runtime::{Binding, ExecutorError},
};

use super::{pattern, visited_set};

pub(crate) fn execute(
    child: &JoinTree,
    match_mode: MatchMode,
    path_contributors: &[PathContributor],
    env: pattern::WalkContext<'_, '_, '_, '_, '_, '_>,
) -> Result<Vec<Binding>, ExecutorError> {
    let child_rows = pattern::walk_join_tree(child, env)?;
    match match_mode {
        // Per ISO 39075:2024 §16.4 GR8(a): DIFFERENT EDGES keeps only the
        // different-edges-matched rows — those whose edge bindings reference
        // pairwise-distinct graph edges across all path patterns. Reuses the
        // shared `trail_allows_path` primitive, which inserts every edge column
        // (named / hidden / questioned / variable-length group) into a set and
        // rejects on the first duplicate `EdgeId`. A violation FILTERS (drops
        // the row); §16.4 attaches no GQLSTATUS, so no error is raised.
        MatchMode::DifferentEdges => {
            let mut selected = Vec::new();
            let mut rows_since_check = 0;
            for row in child_rows {
                env.ctx
                    .tx
                    .check_cancellation_stride(&mut rows_since_check, 1)?;
                if visited_set::trail_allows_path(&row, path_contributors, env)? {
                    selected.push(row);
                }
            }
            Ok(selected)
        }
        // Per ISO 39075:2024 §16.4 GR8(b): REPEATABLE ELEMENTS is BINDINGS =
        // INNER (no filter). Lowering never constructs this wrapper for it, but
        // the runtime stays exhaustive and behaves as a pass-through if it ever
        // does.
        MatchMode::RepeatableElements => Ok(child_rows),
    }
}
