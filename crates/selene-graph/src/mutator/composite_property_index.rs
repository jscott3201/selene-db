//! Composite property-index mutation methods for the transaction mutator.

use selene_core::{Change, IStr, SchemaChange, SchemaPropertyIndexKind};
use smallvec::SmallVec;

use crate::graph::{CompositePropertyIndexEntry, composite_property_key};
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
        label: IStr,
        properties: SmallVec<[IStr; 4]>,
        kinds: SmallVec<[TypedIndexKind; 4]>,
        name: Option<IStr>,
    ) -> GraphResult<()> {
        validate_shape(&properties, &kinds)?;
        let key = composite_property_key(&properties);
        if self
            .txn
            .read()
            .composite_property_index
            .contains_key(&(label, key.clone()))
        {
            return Err(GraphError::CompositePropertyIndexAlreadyExists { label, properties });
        }
        let graph_id = self.txn.read().graph_id();
        let index = crate::composite_property_index::build_composite_property_index(
            self.txn.read(),
            label,
            properties.clone(),
            kinds.clone(),
        )?;
        self.txn.guard_mut().composite_property_index.insert(
            (label, key),
            CompositePropertyIndexEntry::new(index, properties.clone(), name),
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
        label: IStr,
        properties: SmallVec<[IStr; 4]>,
    ) -> GraphResult<()> {
        let key = composite_property_key(&properties);
        if !self
            .txn
            .read()
            .composite_property_index
            .contains_key(&(label, key.clone()))
        {
            return Ok(());
        }
        let graph_id = self.txn.read().graph_id();
        self.txn
            .guard_mut()
            .composite_property_index
            .remove(&(label, key));
        self.txn.changes.push(Change::SchemaChanged {
            graph: graph_id,
            change: SchemaChange::CompositePropertyIndexDropped { label, properties },
        });
        Ok(())
    }
}

fn validate_shape(properties: &[IStr], kinds: &[TypedIndexKind]) -> Result<(), GraphError> {
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
