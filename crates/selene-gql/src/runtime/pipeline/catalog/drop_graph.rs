//! `DROP GRAPH` factory-reset execution (BRIEF-152, deletion-reclamation audit
//! Item 10).
//!
//! Extracted from the catalog pipeline operator to keep `mod.rs` under the
//! 700-LOC file cap, mirroring `drop_cascade`. The mutator (the single write
//! funnel, hard rule 11) owns the wipe-all + schema-reset logic; the executor
//! only threads the planned operation through.
//!
//! Why factory-reset: under D1 the engine is a single-graph embeddable library
//! with no string graph-name registry — `GraphMeta` carries only a `GraphId`
//! and a session is bound to one shared graph. There is nothing to resolve a
//! `DROP GRAPH <ident>` against, so the parsed `name` is informational and the
//! implicit target is always the session graph. `IF EXISTS` is therefore
//! trivially satisfied (the embedded graph always exists in an open session)
//! and never changes behaviour: both `DROP GRAPH g` and `DROP GRAPH IF EXISTS g`
//! perform the same factory-reset (wipe all data + reset schema to open),
//! keeping the MANIFEST epoch and WAL lineage intact. This is forward-compatible
//! with multi-graph (v1.4+), where the name resolves to a real catalog entry.

use selene_core::IStr;

use crate::{
    SourceSpan,
    runtime::{BindingTable, ExecutorError, TxContext},
};

use super::catalog_graph_error;

/// Execute `DROP GRAPH [IF EXISTS] <name>` as a factory-reset of the session
/// graph.
///
/// Wipes every node and edge (including untyped/arbitrary-label rows) and resets
/// the schema to open, recording exactly one declarative `Change::GraphReset`
/// (O(1) WAL). Idempotent: a second `DROP GRAPH` on an already-empty + open
/// graph is a clean no-op, not an error.
pub(super) fn execute_drop_graph(
    name: IStr,
    if_exists: bool,
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    // The parsed name is informational under D1 (single graph), and IF EXISTS is
    // always satisfied — neither alters the target. Discarded explicitly so the
    // intent is clear and the multi-graph forward path has an obvious hook.
    let _ = (name, if_exists);
    ctx.ensure_write_txn("catalog op invoked without write transaction", span)?;
    ctx.mutator_with_span("catalog op invoked without write transaction", span)?
        .factory_reset()
        .map_err(|source| catalog_graph_error(source, span))?;
    Ok(table)
}
