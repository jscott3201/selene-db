//! Vector-index mutation methods for the transaction mutator.

use selene_core::{Change, IStr, SchemaChange, SchemaVectorIndexKind};

use crate::graph::VectorIndexEntry;
use crate::{GraphError, GraphResult, Mutator, VectorIndexKind};

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Register a durable node vector index in the active write transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::VectorIndexAlreadyExists`] if the pair already
    /// exists, [`GraphError::VectorIndexInvalidDimension`] when `dimension` is
    /// zero, or [`GraphError::VectorIndexValueRejected`] if an existing non-null
    /// value for `(label, property)` is not a vector with `dimension`.
    pub fn create_vector_index(
        &mut self,
        label: IStr,
        property: IStr,
        kind: VectorIndexKind,
        dimension: u32,
    ) -> GraphResult<()> {
        self.create_vector_index_named(label, property, kind, dimension, None)
    }

    /// Register a durable node vector index with optional catalog name.
    pub fn create_vector_index_named(
        &mut self,
        label: IStr,
        property: IStr,
        kind: VectorIndexKind,
        dimension: u32,
        name: Option<IStr>,
    ) -> GraphResult<()> {
        if self
            .txn
            .read()
            .vector_index
            .contains_key(&(label.clone(), property.clone()))
        {
            return Err(GraphError::VectorIndexAlreadyExists { label, property });
        }
        let index = crate::vector_index::build_vector_index(
            self.txn.read(),
            label.clone(),
            property.clone(),
            kind,
            dimension,
        )?;
        let graph_id = self.txn.read().graph_id();
        self.txn.guard_mut().vector_index.insert(
            (label.clone(), property.clone()),
            VectorIndexEntry::new(index, name.clone()),
        );
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::VectorIndexCreated {
                label,
                property,
                kind: schema_kind_from(kind),
                dimension,
                name,
            },
        });
        Ok(())
    }

    /// Drop a durable node vector index from the active write transaction.
    ///
    /// The operation is idempotent. Dropping an absent index succeeds and emits
    /// no WAL change.
    pub fn drop_vector_index(&mut self, label: IStr, property: IStr) -> GraphResult<()> {
        if !self
            .txn
            .read()
            .vector_index
            .contains_key(&(label.clone(), property.clone()))
        {
            return Ok(());
        }
        let graph_id = self.txn.read().graph_id();
        self.txn
            .guard_mut()
            .vector_index
            .remove(&(label.clone(), property.clone()));
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::VectorIndexDropped { label, property },
        });
        Ok(())
    }
}

const fn schema_kind_from(kind: VectorIndexKind) -> SchemaVectorIndexKind {
    match kind {
        VectorIndexKind::Flat => SchemaVectorIndexKind::Flat,
    }
}

#[cfg(test)]
#[path = "vector_index/tests.rs"]
mod tests;
