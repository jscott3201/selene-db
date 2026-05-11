//! Procedure-pack history source abstraction.
//!
//! Embedders that want `selene.pack.history` to be queryable register a
//! [`PackHistorySource`] at registry construction time. The trait is the single
//! seam between the procedure and the WAL file the embedder configured at
//! startup.

use selene_gql::ProcedureError;
use selene_persist::WalReader;

/// Embedder-injected source of procedure-pack lifecycle history.
///
/// `selene.pack.history` calls [`PackHistorySource::open_wal_reader`] exactly
/// once per invocation, iterates the returned reader, filters for
/// `Change::SchemaChanged { change: SchemaChange::ProcedurePackLifecycle { .. } }`,
/// and projects each event into a row of the procedure's output schema.
///
/// Implementations are responsible for resolving the active WAL path (for
/// example, from embedder configuration) and for handling concurrent access.
/// Returning an error surfaces as [`ProcedureError::Internal`] to the caller.
///
/// # Query shape
///
/// Phase A exposes no procedure parameters. Callers filter with GQL:
///
/// ```text
/// CALL selene.pack.history() YIELD * WHERE kind = 'activated'
/// ```
///
/// # Read consistency
///
/// `WalReader::iterate` snapshots `file_len` at iteration start. Implementations
/// of `open_wal_reader` are responsible for providing a readable WAL view at the
/// moment of call; consistency under concurrent appends is the source's
/// responsibility. A source may coordinate with the embedder's WAL-writing
/// provider, read from a quiesced file, or accept that the iteration sees only
/// entries committed before the `file_len` snapshot.
///
/// Multiple invocations may observe different tails of the WAL.
pub trait PackHistorySource: Send + Sync + 'static {
    /// Open a fresh [`WalReader`] for one procedure invocation.
    ///
    /// # Errors
    ///
    /// Returns a [`ProcedureError`] when the source cannot provide a readable WAL
    /// view.
    fn open_wal_reader(&self) -> Result<WalReader, ProcedureError>;
}
