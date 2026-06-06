//! Text-index mutation methods for the transaction mutator.

use selene_core::{Change, DbString, SchemaChange};

use crate::graph::TextIndexEntry;
use crate::{GraphError, GraphResult, Mutator, TextIndex};

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Register a durable node text index in the active write transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::TextIndexAlreadyExists`] if the pair already
    /// exists, or [`GraphError::Inconsistent`] if index construction observes
    /// corrupt graph columns.
    pub fn create_text_index(&mut self, label: DbString, property: DbString) -> GraphResult<()> {
        self.create_text_index_named(label, property, None)
    }

    /// Register a durable node text index with optional catalog name.
    pub fn create_text_index_named(
        &mut self,
        label: DbString,
        property: DbString,
        name: Option<DbString>,
    ) -> GraphResult<()> {
        if self
            .txn
            .read()
            .text_index
            .contains_key(&(label.clone(), property.clone()))
        {
            return Err(GraphError::TextIndexAlreadyExists { label, property });
        }
        let index = TextIndex::build(self.txn.read(), label.clone(), property.clone())?;
        let graph_id = self.txn.read().graph_id();
        self.txn.guard_mut().text_index.insert(
            (label.clone(), property.clone()),
            TextIndexEntry::new(index, name.clone()),
        );
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::TextIndexCreated {
                label,
                property,
                name,
            },
        });
        Ok(())
    }

    /// Drop a durable node text index from the active write transaction.
    ///
    /// The operation is idempotent. Dropping an absent index succeeds and emits
    /// no WAL change.
    pub fn drop_text_index(&mut self, label: DbString, property: DbString) -> GraphResult<()> {
        if !self
            .txn
            .read()
            .text_index
            .contains_key(&(label.clone(), property.clone()))
        {
            return Ok(());
        }
        let graph_id = self.txn.read().graph_id();
        self.txn
            .guard_mut()
            .text_index
            .remove(&(label.clone(), property.clone()));
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::TextIndexDropped { label, property },
        });
        Ok(())
    }
}

#[cfg(test)]
#[path = "text_index/tests.rs"]
mod tests;
