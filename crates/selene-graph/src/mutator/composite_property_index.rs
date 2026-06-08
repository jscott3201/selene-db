//! Composite property-index mutation methods for the transaction mutator.

use selene_core::{Change, DbString, SchemaChange, SchemaPropertyIndexKind};
use smallvec::SmallVec;

use crate::graph::{CompositePropertyIndexEntry, composite_property_key};
use crate::schema_index_kind::schema_kind_from;
use crate::{GraphError, GraphResult, Mutator, TypedIndexKind};

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Register a durable node composite-property index with optional catalog name.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::CompositePropertyIndexAlreadyExists`] if the
    /// canonical property set already exists.
    pub fn create_composite_property_index_named(
        &mut self,
        label: DbString,
        properties: SmallVec<[DbString; 4]>,
        kinds: SmallVec<[TypedIndexKind; 4]>,
        name: Option<DbString>,
    ) -> GraphResult<()> {
        validate_shape(&properties, &kinds)?;
        let key = composite_property_key(&properties);
        if self
            .txn
            .read()
            .composite_property_index
            .contains_key(&(label.clone(), key.clone()))
        {
            return Err(GraphError::CompositePropertyIndexAlreadyExists {
                label,
                properties: Box::new(properties),
            });
        }
        let graph_id = self.txn.read().graph_id();
        let index = crate::composite_property_index::build_composite_property_index(
            self.txn.read(),
            label.clone(),
            properties.clone(),
            kinds.clone(),
        )?;
        self.txn.guard_mut().composite_property_index.insert(
            (label.clone(), key),
            CompositePropertyIndexEntry::new(index, properties.clone(), name.clone()),
        );
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::CompositePropertyIndexCreated {
                label,
                properties,
                kinds: schema_kinds_from(&kinds),
                name,
            },
        });
        Ok(())
    }

    /// Drop a durable node composite-property index from the active write transaction.
    ///
    /// The operation is idempotent. Dropping an absent index succeeds and emits
    /// no WAL change.
    pub fn drop_composite_property_index(
        &mut self,
        label: DbString,
        properties: SmallVec<[DbString; 4]>,
    ) -> GraphResult<()> {
        let key = composite_property_key(&properties);
        if !self
            .txn
            .read()
            .composite_property_index
            .contains_key(&(label.clone(), key.clone()))
        {
            return Ok(());
        }
        let graph_id = self.txn.read().graph_id();
        self.txn
            .guard_mut()
            .composite_property_index
            .remove(&(label.clone(), key));
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::CompositePropertyIndexDropped { label, properties },
        });
        Ok(())
    }
}

fn validate_shape(properties: &[DbString], kinds: &[TypedIndexKind]) -> Result<(), GraphError> {
    if properties.len() < 2 {
        return Err(GraphError::Inconsistent {
            reason: "composite index requires at least two properties".to_owned(),
        });
    }
    if properties.len() != kinds.len() {
        return Err(GraphError::Inconsistent {
            reason: format!(
                "composite index has {} properties but {} kinds",
                properties.len(),
                kinds.len()
            ),
        });
    }
    let mut key = properties.to_vec();
    key.sort();
    key.dedup();
    if key.len() != properties.len() {
        return Err(GraphError::Inconsistent {
            reason: "composite index property list contains duplicates".to_owned(),
        });
    }
    Ok(())
}

fn schema_kinds_from(kinds: &[TypedIndexKind]) -> SmallVec<[SchemaPropertyIndexKind; 4]> {
    kinds.iter().copied().map(schema_kind_from).collect()
}

#[cfg(test)]
mod tests {
    use selene_core::{GraphId, db_string};
    use smallvec::smallvec;

    use crate::{GraphError, SharedGraph, TypedIndexKind};

    #[test]
    fn create_composite_property_index_rejects_empty_property_list() {
        let shared = SharedGraph::new(GraphId::new(140_201));
        let mut txn = shared.begin_write();
        let err = txn
            .mutator()
            .create_composite_property_index_named(
                db_string("CompositeShape").unwrap(),
                smallvec![],
                smallvec![],
                None,
            )
            .unwrap_err();

        assert!(matches!(
            err,
            GraphError::Inconsistent { reason }
                if reason == "composite index requires at least two properties"
        ));
    }

    #[test]
    fn create_composite_property_index_rejects_single_property_list() {
        let shared = SharedGraph::new(GraphId::new(140_202));
        let mut txn = shared.begin_write();
        let err = txn
            .mutator()
            .create_composite_property_index_named(
                db_string("CompositeShape").unwrap(),
                smallvec![db_string("only").unwrap()],
                smallvec![TypedIndexKind::I64],
                None,
            )
            .unwrap_err();

        assert!(matches!(
            err,
            GraphError::Inconsistent { reason }
                if reason == "composite index requires at least two properties"
        ));
    }
}
