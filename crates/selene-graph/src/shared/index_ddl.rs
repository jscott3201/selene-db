//! Shared graph index-DDL wrappers.

use selene_core::{Change, DbString, HnswIndexConfig, SchemaChange};

use super::SharedGraph;
use crate::error::{GraphError, GraphResult};
use crate::graph::PropertyIndexEntry;
use crate::schema_index_kind::schema_kind_from;
use crate::typed_index::TypedIndexKind;
use crate::vector_index::{VectorIndexConfig, VectorIndexKind};

impl SharedGraph {
    /// Register a built-in node property index for `(label, property)`.
    ///
    /// The current node columns are scanned under the write lock and the
    /// published snapshot is updated in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::PropertyIndexAlreadyExists`] if the pair is
    /// already registered, or [`GraphError::IndexValueRejected`] if any
    /// existing node with `label` has a non-null value that does not match
    /// `kind`.
    pub fn create_property_index(
        &self,
        label: DbString,
        property: DbString,
        kind: TypedIndexKind,
    ) -> GraphResult<()> {
        self.create_property_index_named(label, property, kind, None)
    }

    /// Register a built-in node property index with optional catalog name.
    pub fn create_property_index_named(
        &self,
        label: DbString,
        property: DbString,
        kind: TypedIndexKind,
        name: Option<DbString>,
    ) -> GraphResult<()> {
        let mut txn = self.begin_write();
        if txn
            .read()
            .property_index
            .contains_key(&(label.clone(), property.clone()))
        {
            return Err(GraphError::PropertyIndexAlreadyExists { label, property });
        }
        let index = crate::property_index::build_property_index(
            txn.read(),
            label.clone(),
            property.clone(),
            kind,
        )?;
        txn.guard_mut().property_index.insert(
            (label.clone(), property.clone()),
            PropertyIndexEntry::new(index, name.clone()),
        );
        let graph = txn.read().graph_id();
        txn.changes.push(Change::SchemaChanged {
            graph,
            change: SchemaChange::PropertyIndexCreatedNamed {
                label,
                property,
                kind: schema_kind_from(kind),
                name,
            },
        });
        txn.commit()?;
        Ok(())
    }

    /// Drop a built-in node property index.
    ///
    /// The operation is idempotent; dropping an absent index succeeds without
    /// publishing a new snapshot.
    pub fn drop_property_index(&self, label: DbString, property: DbString) -> GraphResult<()> {
        let mut txn = self.begin_write();
        if !txn
            .read()
            .property_index
            .contains_key(&(label.clone(), property.clone()))
        {
            return Ok(());
        }
        txn.guard_mut()
            .property_index
            .remove(&(label.clone(), property.clone()));
        let graph = txn.read().graph_id();
        txn.changes.push(Change::SchemaChanged {
            graph,
            change: SchemaChange::PropertyIndexDropped { label, property },
        });
        txn.commit()?;
        Ok(())
    }

    /// Register a built-in node vector index for `(label, property)`.
    ///
    /// The current node columns are scanned under the write lock and the
    /// published snapshot is updated in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::VectorIndexAlreadyExists`] if the pair is already
    /// registered, [`GraphError::VectorIndexInvalidDimension`] when `dimension`
    /// is zero, or [`GraphError::VectorIndexValueRejected`] if any existing
    /// node with `label` has a non-null value for `property` that is not a
    /// vector with the declared dimension.
    pub fn create_vector_index(
        &self,
        label: DbString,
        property: DbString,
        kind: VectorIndexKind,
        dimension: u32,
    ) -> GraphResult<()> {
        self.create_vector_index_named(label, property, kind, dimension, None)
    }

    /// Register a built-in node vector index with optional catalog name.
    pub fn create_vector_index_named(
        &self,
        label: DbString,
        property: DbString,
        kind: VectorIndexKind,
        dimension: u32,
        name: Option<DbString>,
    ) -> GraphResult<()> {
        self.create_vector_index_named_with_config(label, property, kind, dimension, name, None)
    }

    /// Register a built-in node vector index with optional HNSW construction config.
    pub fn create_vector_index_named_with_config(
        &self,
        label: DbString,
        property: DbString,
        kind: VectorIndexKind,
        dimension: u32,
        name: Option<DbString>,
        hnsw_config: Option<HnswIndexConfig>,
    ) -> GraphResult<()> {
        self.create_vector_index_named_with_configs(
            label,
            property,
            kind,
            dimension,
            name,
            VectorIndexConfig::new(hnsw_config, None),
        )
    }

    /// Register a built-in node vector index with optional ANN construction config.
    pub fn create_vector_index_named_with_configs(
        &self,
        label: DbString,
        property: DbString,
        kind: VectorIndexKind,
        dimension: u32,
        name: Option<DbString>,
        config: VectorIndexConfig,
    ) -> GraphResult<()> {
        let mut txn = self.begin_write();
        txn.mutator().create_vector_index_named_with_configs(
            label, property, kind, dimension, name, config,
        )?;
        txn.commit()?;
        Ok(())
    }

    /// Drop a built-in node vector index.
    ///
    /// The operation is idempotent; dropping an absent index succeeds without
    /// publishing a new snapshot.
    pub fn drop_vector_index(&self, label: DbString, property: DbString) -> GraphResult<()> {
        let mut txn = self.begin_write();
        txn.mutator().drop_vector_index(label, property)?;
        txn.commit()?;
        Ok(())
    }

    /// Register a built-in node text index for `(label, property)`.
    ///
    /// The current node columns are scanned under the write lock and the
    /// published snapshot is updated in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::TextIndexAlreadyExists`] if the pair is already
    /// registered, or [`GraphError::Inconsistent`] if index construction
    /// observes corrupt graph columns.
    pub fn create_text_index(&self, label: DbString, property: DbString) -> GraphResult<()> {
        self.create_text_index_named(label, property, None)
    }

    /// Register a built-in node text index with optional catalog name.
    pub fn create_text_index_named(
        &self,
        label: DbString,
        property: DbString,
        name: Option<DbString>,
    ) -> GraphResult<()> {
        let mut txn = self.begin_write();
        txn.mutator()
            .create_text_index_named(label, property, name)?;
        txn.commit()?;
        Ok(())
    }

    /// Drop a built-in node text index.
    ///
    /// The operation is idempotent; dropping an absent index succeeds without
    /// publishing a new snapshot.
    pub fn drop_text_index(&self, label: DbString, property: DbString) -> GraphResult<()> {
        let mut txn = self.begin_write();
        txn.mutator().drop_text_index(label, property)?;
        txn.commit()?;
        Ok(())
    }
}
