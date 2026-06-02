//! Match-mode lowering helpers.
//!
//! Per ISO/IEC 39075:2024 §16.4, a `<match mode>` prefixes the entire
//! comma-separated path-pattern list of one MATCH. It is orthogonal to the
//! per-path-pattern `<path mode>` ([`super::path_mode`]): DIFFERENT EDGES
//! (Feature G002) enforces pattern-wide edge uniqueness across every path
//! pattern jointly, while REPEATABLE ELEMENTS (Feature G003) and the
//! implementation-defined default install no filter.

use crate::{
    MatchMode, SourceSpan,
    plan::{JoinTree, PlannerError},
};

use super::path_mode::collect_path_contributors;

/// Wrap a fully-joined graph pattern in a pattern-wide match-mode filter.
///
/// `child` is the join of all comma-separated path patterns of one MATCH
/// clause, so its contributor union is the pattern-wide edge set ISO §16.4 GR4
/// describes. Only [`MatchMode::DifferentEdges`] installs a filter:
///
/// - **`DifferentEdges`** (§16.4 GR8(a)): wrap in [`JoinTree::MatchModeFilter`]
///   carrying every edge contributor across all path patterns. The runtime
///   drops any row that binds one graph edge to two positions.
/// - **`RepeatableElements`** (§16.4 GR8(b): `BINDINGS = INNER`): no filter;
///   return `child` unchanged.
/// - **`None`** (implementation-defined default ID086 = REPEATABLE ELEMENTS):
///   no filter, identical to an explicit REPEATABLE ELEMENTS.
pub(super) fn wrap_in_match_mode_filter(
    child: JoinTree,
    match_mode: Option<MatchMode>,
    span: SourceSpan,
) -> Result<JoinTree, PlannerError> {
    // Per ISO 39075:2024 §16.4 SR10: when no `<match mode>` is written, an
    // implementation-defined (ID086) mode is implicit. selene's ID086 default is
    // REPEATABLE ELEMENTS — selected because selene's pre-812 no-prefix MATCH
    // installs no pattern-wide edge filter (that IS REPEATABLE ELEMENTS), so
    // declaring the default is documentation with zero behavior change to any
    // existing query. Only an explicit DIFFERENT EDGES is therefore opt-in.
    let Some(MatchMode::DifferentEdges) = match_mode else {
        return Ok(child);
    };
    let mut path_contributors = Vec::new();
    collect_path_contributors(&child, span, &mut path_contributors)?;
    Ok(JoinTree::MatchModeFilter {
        match_mode: MatchMode::DifferentEdges,
        child: Box::new(child),
        path_contributors,
    })
}
