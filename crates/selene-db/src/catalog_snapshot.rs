//! Facade descriptor summaries and immutable outer read snapshots.

use std::sync::Arc;

use selene_catalog::{
    CatalogDescriptor as LowerDescriptor, CatalogObjectId, CatalogObjectKind, CatalogParent,
    CatalogPayload, GraphId as LowerGraphId, GraphTypeId as LowerGraphTypeId,
    SchemaId as LowerSchemaId,
};

use crate::{Error, ObjectPath, PathSegment, Result, SchemaPath, database::DatabaseState};

macro_rules! facade_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub(crate) u64);

        impl $name {
            /// Return the stable nonzero numeric identity.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

facade_id!(SchemaId, "Stable facade schema identity.");
facade_id!(GraphId, "Stable facade graph identity.");
facade_id!(GraphTypeId, "Stable facade graph-type identity.");

/// Generation of one atomically published database catalog state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CatalogGeneration(u64);

impl CatalogGeneration {
    pub(crate) const fn from_lower(generation: selene_catalog::CatalogGeneration) -> Self {
        Self(generation.get())
    }

    #[cfg(test)]
    pub(crate) const fn from_raw(generation: u64) -> Self {
        Self(generation)
    }

    /// Return the nonzero generation number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Return the checked next generation, or `None` at `u64::MAX`.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

/// Facade summary of a schema descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaDescriptor {
    /// Stable schema identity.
    pub id: SchemaId,
    /// Absolute logical path.
    pub path: SchemaPath,
    /// Descriptor creation generation.
    pub created_at: CatalogGeneration,
}

/// Facade summary of a graph descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDescriptor {
    /// Stable graph identity.
    pub id: GraphId,
    /// Absolute logical path.
    pub path: ObjectPath,
    /// Optional constraining graph-type identity.
    pub graph_type: Option<GraphTypeId>,
    /// Descriptor creation generation.
    pub created_at: CatalogGeneration,
}

/// Facade summary of a graph-type descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphTypeDescriptor {
    /// Stable graph-type identity.
    pub id: GraphTypeId,
    /// Absolute logical path.
    pub path: ObjectPath,
    /// Number of declared node types.
    pub node_type_count: usize,
    /// Descriptor creation generation.
    pub created_at: CatalogGeneration,
}

/// O(1) immutable view of one complete outer database publication.
///
/// Retaining this value also retains that publication's runtime graph and
/// graph-type allocations. Logical drops remove them from newer snapshots and
/// handles, but reclamation waits for older read snapshots to be released.
#[derive(Clone)]
pub struct CatalogReadSnapshot {
    pub(crate) state: Arc<DatabaseState>,
}

impl CatalogReadSnapshot {
    /// Return this publication's catalog generation.
    #[must_use]
    pub fn generation(&self) -> CatalogGeneration {
        CatalogGeneration(self.state.catalog.generation().get())
    }

    /// Return whether two snapshots share the same outer state allocation.
    #[must_use]
    pub fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    /// Resolve a schema by absolute path.
    pub fn resolve_schema(&self, path: &SchemaPath) -> Result<SchemaDescriptor> {
        ensure_catalog(&self.state, path.catalog())?;
        let descriptor = self
            .state
            .catalog
            .schema(&path.schema.0)
            .ok_or_else(|| Error::not_found(path, "schema"))?;
        schema_summary(&self.state, descriptor)
    }

    /// Resolve a graph by absolute path.
    pub fn resolve_graph(&self, path: &ObjectPath) -> Result<GraphDescriptor> {
        let descriptor = require_object(&self.state, path, CatalogObjectKind::Graph, "graph")?;
        graph_summary(&self.state, descriptor)
    }

    /// Resolve a graph type by absolute path.
    pub fn resolve_graph_type(&self, path: &ObjectPath) -> Result<GraphTypeDescriptor> {
        let descriptor = require_object(
            &self.state,
            path,
            CatalogObjectKind::GraphType,
            "graph type",
        )?;
        graph_type_summary(&self.state, descriptor)
    }

    /// List schemas by canonical name and stable ID.
    pub fn schemas(&self) -> Result<Vec<SchemaDescriptor>> {
        let mut summaries = self
            .state
            .catalog
            .schemas()
            .map(|descriptor| schema_summary(&self.state, descriptor))
            .collect::<Result<Vec<_>>>()?;
        summaries.sort_by(|left, right| {
            left.path
                .schema()
                .cmp(right.path.schema())
                .then(left.id.cmp(&right.id))
        });
        Ok(summaries)
    }

    /// List graphs in a schema by canonical name and stable ID.
    pub fn graphs(&self, schema: &SchemaPath) -> Result<Vec<GraphDescriptor>> {
        let schema_id = require_schema(&self.state, schema)?;
        let mut summaries = self
            .state
            .catalog
            .schema_objects(schema_id)
            .into_iter()
            .flatten()
            .filter(|descriptor| descriptor.kind() == CatalogObjectKind::Graph)
            .map(|descriptor| graph_summary(&self.state, descriptor))
            .collect::<Result<Vec<_>>>()?;
        summaries.sort_by(|left, right| {
            left.path
                .object()
                .cmp(right.path.object())
                .then(left.id.cmp(&right.id))
        });
        Ok(summaries)
    }

    /// List graph types in a schema by canonical name and stable ID.
    pub fn graph_types(&self, schema: &SchemaPath) -> Result<Vec<GraphTypeDescriptor>> {
        let schema_id = require_schema(&self.state, schema)?;
        let mut summaries = self
            .state
            .catalog
            .schema_objects(schema_id)
            .into_iter()
            .flatten()
            .filter(|descriptor| descriptor.kind() == CatalogObjectKind::GraphType)
            .map(|descriptor| graph_type_summary(&self.state, descriptor))
            .collect::<Result<Vec<_>>>()?;
        summaries.sort_by(|left, right| {
            left.path
                .object()
                .cmp(right.path.object())
                .then(left.id.cmp(&right.id))
        });
        Ok(summaries)
    }

    pub(crate) fn matches_schema_reference(&self, expected: &SchemaDescriptor) -> bool {
        let Ok(id) = LowerSchemaId::new(expected.id.0) else {
            return false;
        };
        self.state
            .catalog
            .descriptor(CatalogObjectId::Schema(id))
            .and_then(|descriptor| schema_summary(&self.state, descriptor).ok())
            .as_ref()
            == Some(expected)
    }

    pub(crate) fn matches_graph_reference(&self, expected: &GraphDescriptor) -> bool {
        let Ok(id) = LowerGraphId::new(expected.id.0) else {
            return false;
        };
        self.state
            .catalog
            .descriptor(CatalogObjectId::Graph(id))
            .and_then(|descriptor| graph_summary(&self.state, descriptor).ok())
            .as_ref()
            == Some(expected)
    }

    pub(crate) fn graph_by_id(&self, id: GraphId) -> Result<Option<GraphDescriptor>> {
        let lower = LowerGraphId::new(id.0).map_err(Error::from_catalog_invariant)?;
        self.state
            .catalog
            .descriptor(CatalogObjectId::Graph(lower))
            .map(|descriptor| graph_summary(&self.state, descriptor))
            .transpose()
    }
}

pub(crate) fn next_id(current: u64, kind: &'static str) -> Result<u64> {
    current
        .checked_add(1)
        .ok_or_else(|| Error::identifier_exhausted(kind))
}

pub(crate) fn ensure_catalog(state: &DatabaseState, segment: &PathSegment) -> Result<()> {
    let descriptor = state
        .catalog
        .descriptor(CatalogObjectId::Catalog(state.catalog.catalog_id()))
        .ok_or_else(|| Error::catalog_invariant("published catalog descriptor is missing"))?;
    if descriptor.name() == &segment.0 {
        Ok(())
    } else {
        Err(Error::not_found(segment, "catalog"))
    }
}

pub(crate) fn require_schema(state: &DatabaseState, path: &SchemaPath) -> Result<LowerSchemaId> {
    ensure_catalog(state, path.catalog())?;
    let descriptor = state
        .catalog
        .schema(&path.schema.0)
        .ok_or_else(|| Error::not_found(path, "schema"))?;
    let CatalogObjectId::Schema(id) = descriptor.id() else {
        unreachable!("schema dictionary contains schemas")
    };
    Ok(id)
}

pub(crate) fn find_object<'a>(
    state: &'a DatabaseState,
    path: &ObjectPath,
) -> Result<Option<&'a LowerDescriptor>> {
    let schema = require_schema(state, &path.schema_path())?;
    Ok(state.catalog.schema_object(schema, &path.object.0))
}

pub(crate) fn require_object<'a>(
    state: &'a DatabaseState,
    path: &ObjectPath,
    expected: CatalogObjectKind,
    expected_name: &'static str,
) -> Result<&'a LowerDescriptor> {
    let descriptor =
        find_object(state, path)?.ok_or_else(|| Error::not_found(path, expected_name))?;
    if descriptor.kind() != expected {
        return Err(Error::wrong_kind(path, expected_name, descriptor.kind()));
    }
    Ok(descriptor)
}

pub(crate) fn resolve_binding(
    state: &DatabaseState,
    schema: LowerSchemaId,
    path: Option<&ObjectPath>,
) -> Result<(
    Option<LowerGraphTypeId>,
    Option<Arc<selene_graph::GraphTypeDef>>,
)> {
    let Some(path) = path else {
        return Ok((None, None));
    };
    let binding_schema = require_schema(state, &path.schema_path())?;
    if binding_schema != schema {
        return Err(Error::cross_schema_graph_type(path));
    }
    let descriptor = require_object(state, path, CatalogObjectKind::GraphType, "graph type")?;
    let CatalogObjectId::GraphType(id) = descriptor.id() else {
        unreachable!("kind and typed ID agree")
    };
    let runtime = state.graph_types.get(&id).cloned().ok_or_else(|| {
        Error::catalog_invariant("graph-type descriptor has no runtime definition")
    })?;
    Ok((Some(id), Some(runtime)))
}

pub(crate) fn schema_summary(
    state: &DatabaseState,
    descriptor: &LowerDescriptor,
) -> Result<SchemaDescriptor> {
    let CatalogObjectId::Schema(id) = descriptor.id() else {
        return Err(Error::catalog_invariant(
            "schema summary received another kind",
        ));
    };
    Ok(SchemaDescriptor {
        id: SchemaId(id.get()),
        path: SchemaPath::new(
            catalog_segment(state)?,
            PathSegment(descriptor.name().clone()),
        ),
        created_at: CatalogGeneration(descriptor.creation().generation().get()),
    })
}

pub(crate) fn graph_summary(
    state: &DatabaseState,
    descriptor: &LowerDescriptor,
) -> Result<GraphDescriptor> {
    let CatalogObjectId::Graph(id) = descriptor.id() else {
        return Err(Error::catalog_invariant(
            "graph summary received another kind",
        ));
    };
    let CatalogPayload::Graph { graph_type } = descriptor.payload() else {
        return Err(Error::catalog_invariant(
            "graph descriptor payload is invalid",
        ));
    };
    Ok(GraphDescriptor {
        id: GraphId(id.get()),
        path: object_path(state, descriptor)?,
        graph_type: graph_type.map(|id| GraphTypeId(id.get())),
        created_at: CatalogGeneration(descriptor.creation().generation().get()),
    })
}

pub(crate) fn graph_type_summary(
    state: &DatabaseState,
    descriptor: &LowerDescriptor,
) -> Result<GraphTypeDescriptor> {
    let CatalogObjectId::GraphType(id) = descriptor.id() else {
        return Err(Error::catalog_invariant(
            "graph-type summary received another kind",
        ));
    };
    let runtime = state.graph_types.get(&id).ok_or_else(|| {
        Error::catalog_invariant("graph-type descriptor has no runtime definition")
    })?;
    Ok(GraphTypeDescriptor {
        id: GraphTypeId(id.get()),
        path: object_path(state, descriptor)?,
        node_type_count: runtime.node_types.len(),
        created_at: CatalogGeneration(descriptor.creation().generation().get()),
    })
}

fn catalog_segment(state: &DatabaseState) -> Result<PathSegment> {
    state
        .catalog
        .descriptor(CatalogObjectId::Catalog(state.catalog.catalog_id()))
        .map(|descriptor| PathSegment(descriptor.name().clone()))
        .ok_or_else(|| Error::catalog_invariant("published catalog descriptor is missing"))
}

fn object_path(state: &DatabaseState, descriptor: &LowerDescriptor) -> Result<ObjectPath> {
    let CatalogParent::Schema(schema) = descriptor.parent() else {
        return Err(Error::catalog_invariant(
            "object summary has no schema parent",
        ));
    };
    let schema = state
        .catalog
        .descriptor(CatalogObjectId::Schema(schema))
        .ok_or_else(|| Error::catalog_invariant("object summary schema is missing"))?;
    Ok(ObjectPath::new(
        catalog_segment(state)?,
        PathSegment(schema.name().clone()),
        PathSegment(descriptor.name().clone()),
    ))
}

#[cfg(test)]
mod tests {
    use super::CatalogGeneration;

    #[test]
    fn catalog_generation_transition_is_checked() {
        assert_eq!(
            CatalogGeneration::from_raw(7).checked_next(),
            Some(CatalogGeneration::from_raw(8))
        );
        assert_eq!(CatalogGeneration::from_raw(u64::MAX).checked_next(), None);
    }
}
