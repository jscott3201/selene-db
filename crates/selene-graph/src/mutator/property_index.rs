//! Property-index mutation methods for the transaction mutator.

use selene_core::{Change, IStr, SchemaChange, SchemaPropertyIndexKind};

use crate::graph::PropertyIndexEntry;
use crate::{GraphError, GraphResult, Mutator, TypedIndexKind};

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Register a durable node property index in the active write transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::PropertyIndexAlreadyExists`] if the pair already
    /// exists, or [`GraphError::IndexValueRejected`] if any existing non-null
    /// value for `(label, property)` cannot be admitted to `kind`.
    pub fn create_property_index(
        &mut self,
        label: IStr,
        property: IStr,
        kind: TypedIndexKind,
    ) -> GraphResult<()> {
        self.create_property_index_named(label, property, kind, None)
    }

    /// Register a durable node property index with optional catalog name.
    pub fn create_property_index_named(
        &mut self,
        label: IStr,
        property: IStr,
        kind: TypedIndexKind,
        name: Option<IStr>,
    ) -> GraphResult<()> {
        if self
            .txn
            .read()
            .property_index
            .contains_key(&(label, property))
        {
            return Err(GraphError::PropertyIndexAlreadyExists { label, property });
        }
        let index =
            crate::property_index::build_property_index(self.txn.read(), label, property, kind)?;
        let graph_id = self.txn.read().graph_id();
        self.txn
            .guard_mut()
            .property_index
            .insert((label, property), PropertyIndexEntry::new(index, name));
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::PropertyIndexCreatedNamed {
                label,
                property,
                kind: schema_kind_from(kind),
                name,
            },
        });
        Ok(())
    }

    /// Drop a durable node property index from the active write transaction.
    ///
    /// The operation is idempotent. Dropping an absent index succeeds and emits
    /// no WAL change.
    pub fn drop_property_index(&mut self, label: IStr, property: IStr) -> GraphResult<()> {
        if !self
            .txn
            .read()
            .property_index
            .contains_key(&(label, property))
        {
            return Ok(());
        }
        let graph_id = self.txn.read().graph_id();
        self.txn
            .guard_mut()
            .property_index
            .remove(&(label, property));
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::PropertyIndexDropped { label, property },
        });
        Ok(())
    }
}

const fn schema_kind_from(kind: TypedIndexKind) -> SchemaPropertyIndexKind {
    match kind {
        TypedIndexKind::I64 => SchemaPropertyIndexKind::I64,
        TypedIndexKind::F64 => SchemaPropertyIndexKind::F64,
        TypedIndexKind::String => SchemaPropertyIndexKind::String,
        TypedIndexKind::Date => SchemaPropertyIndexKind::Date,
        TypedIndexKind::LocalDateTime => SchemaPropertyIndexKind::LocalDateTime,
        TypedIndexKind::Uuid => SchemaPropertyIndexKind::Uuid,
    }
}

#[cfg(test)]
#[path = "property_index/tests.rs"]
mod tests;
