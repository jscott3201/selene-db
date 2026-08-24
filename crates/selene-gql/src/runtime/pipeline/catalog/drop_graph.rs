//! `DROP GRAPH` factory-reset execution (BRIEF-152, deletion-reclamation audit
//! Item 10).
//!
//! Extracted from the catalog pipeline operator to keep `mod.rs` under the
//! 700-LOC file cap, mirroring `drop_cascade`. The mutator (the single write
//! funnel, hard rule 11) owns the wipe-all + schema-reset logic; the executor
//! only threads the planned operation through.
//!
//! Why factory-reset: a bare engine session is bound to one shared graph and
//! has no catalog to resolve a `DROP GRAPH <reference>` against, so the
//! reference is informational and the implicit target is always the session
//! graph. `IF EXISTS` is trivially satisfied and never changes behaviour: both
//! `DROP GRAPH g` and `DROP GRAPH IF EXISTS g` perform the same factory-reset
//! (wipe all data + reset schema to open), keeping the MANIFEST epoch and WAL
//! lineage intact.
//!
//! Named-graph `DROP GRAPH` is executed by the database facade through the
//! catalog service and never reaches this operator. The facade routes only a
//! reference that resolves to the protected bootstrap graph here; M02-PR05
//! owns deleting that bridge together with the bootstrap catalog.

use crate::{
    SourceSpan,
    runtime::{BindingTable, ExecutorError, TxContext},
};

use super::catalog_graph_error;

/// Execute `DROP GRAPH [IF EXISTS] <reference>` as a factory-reset of the
/// session graph.
///
/// Wipes every node and edge (including untyped/arbitrary-label rows) and resets
/// the schema to open, recording exactly one declarative `Change::GraphReset`
/// (O(1) WAL). Idempotent: a second `DROP GRAPH` on an already-empty + open
/// graph is a clean no-op, not an error.
pub(super) fn execute_drop_graph(
    span: SourceSpan,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    ctx.ensure_write_txn("catalog op invoked without write transaction", span)?;
    ctx.mutator_with_span("catalog op invoked without write transaction", span)?
        .factory_reset()
        .map_err(|source| catalog_graph_error(source, span))?;
    Ok(table)
}
