//! Database-owned catalog lifecycle service and immutable read snapshots.

use std::sync::Arc;

use selene_catalog::{
    CatalogDescriptor as LowerDescriptor, CatalogObjectId, CatalogObjectKind, CatalogPayload,
    CatalogTransaction, CreationMetadata, GraphId as LowerGraphId, GraphTypeId as LowerGraphTypeId,
    SchemaId as LowerSchemaId,
};
use selene_core::GraphId as CoreGraphId;
use selene_graph::SharedGraph;

use crate::{
    CatalogReadSnapshot, Error, GraphDescriptor, GraphTypeDefinition, GraphTypeDescriptor,
    ObjectPath, Result, SchemaDescriptor, SchemaPath,
    catalog_snapshot::{
        ensure_catalog, find_object, graph_summary, graph_type_summary, next_id, require_schema,
        resolve_binding, schema_summary,
    },
    database::{DatabaseInner, DatabaseState, GraphInstance},
};

/// Duplicate handling for a create operation.
///
/// This enum grows as ISO catalog statements are implemented; match with a
/// wildcard arm when only some policies matter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CreatePolicy {
    /// Report an existing object as an error.
    #[default]
    Strict,
    /// Return [`CreateOutcome::AlreadyExists`] for the requested kind only.
    IfNotExists,
    /// Drop an existing object of the requested kind and create the new one in
    /// the same publication (ISO/IEC 39075:2024 sections 12.4 and 12.6).
    ///
    /// [`Catalog::create_graph`] applies the full graph-drop admission.
    /// [`Catalog::create_graph_type`] applies `DROP GRAPH TYPE` RESTRICT and
    /// refuses a referenced type. `CREATE SCHEMA` has no `OR REPLACE` form.
    OrReplace,
}

/// Missing-object handling for a drop operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DropPolicy {
    /// Report an absent object as an error.
    #[default]
    Strict,
    /// Return [`DropOutcome::NotFound`] without publishing state.
    IfExists,
}

/// Explicit result of a create request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateOutcome<T> {
    /// A new object was published.
    Created(T),
    /// The same requested kind already existed at the canonical path.
    AlreadyExists(T),
    /// [`CreatePolicy::OrReplace`] dropped the existing object and published
    /// the new one in a single state swap. The two descriptors never share an
    /// identity.
    Replaced {
        /// The descriptor removed by the replacement.
        dropped: T,
        /// The descriptor published in its place.
        created: T,
    },
}

/// Explicit result of a drop request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DropOutcome<T> {
    /// The object was removed from the published state.
    Dropped(T),
    /// `IF EXISTS` found no object at the path.
    NotFound,
}

/// Shared handle to the database catalog service.
#[derive(Clone)]
pub struct Catalog {
    inner: Arc<DatabaseInner>,
}

impl Catalog {
    pub(crate) const fn new(inner: Arc<DatabaseInner>) -> Self {
        Self { inner }
    }

    /// Load the current immutable outer database state in O(1).
    #[must_use]
    pub fn snapshot(&self) -> CatalogReadSnapshot {
        CatalogReadSnapshot {
            state: self.inner.state.load_full(),
        }
    }

    /// Create a root-owned schema.
    pub fn create_schema(
        &self,
        path: &SchemaPath,
        policy: CreatePolicy,
    ) -> Result<CreateOutcome<SchemaDescriptor>> {
        reject_replace(policy, "schema")?;
        let _writer = self.inner.lock_lifecycle_writer();
        let base = self.inner.state.load_full();
        ensure_catalog(&base, path.catalog())?;
        if let Some(existing) = base.catalog.schema(&path.schema.0) {
            let summary = schema_summary(&base, existing)?;
            return duplicate_outcome(policy, summary, path, "schema");
        }
        let raw = next_id(base.high_water.schema, "schema")?;
        let id = LowerSchemaId::new(raw).map_err(Error::from_catalog_invariant)?;
        let mut transaction =
            CatalogTransaction::new(&base.catalog).map_err(Error::from_catalog_invariant)?;
        let descriptor = LowerDescriptor::schema(
            id,
            path.schema.0.clone(),
            base.catalog.root_directory_id(),
            transaction.generation(),
            CreationMetadata::new(transaction.generation(), None),
        )
        .map_err(Error::from_catalog_invariant)?;
        transaction
            .insert(descriptor)
            .map_err(Error::from_catalog_invariant)?;
        self.inner.after_descriptor_staging()?;
        let catalog = transaction.build().map_err(Error::from_catalog_invariant)?;
        let mut high_water = base.high_water;
        high_water.schema = raw;
        let next = Arc::new(DatabaseState {
            catalog,
            graphs: base.graphs.clone(),
            graph_types: base.graph_types.clone(),
            high_water,
        });
        let created = created_summary(&next, CatalogObjectId::Schema(id), schema_summary)?;
        self.inner.before_publication()?;
        self.inner.state.store(next);
        Ok(CreateOutcome::Created(created))
    }

    /// Create and validate a closed graph type.
    pub fn create_graph_type(
        &self,
        path: &ObjectPath,
        definition: GraphTypeDefinition,
        policy: CreatePolicy,
    ) -> Result<CreateOutcome<GraphTypeDescriptor>> {
        let _writer = self.inner.lock_lifecycle_writer();
        let base = self.inner.state.load_full();
        let schema = require_schema(&base, &path.schema_path())?;
        let mut replaced = None;
        if let Some(existing) = base.catalog.schema_object(schema, &path.object.0) {
            if existing.kind() != CatalogObjectKind::GraphType {
                return Err(Error::wrong_kind(path, "graph type", existing.kind()));
            }
            let summary = graph_type_summary(&base, existing)?;
            if policy != CreatePolicy::OrReplace {
                return duplicate_outcome(policy, summary, path, "graph type");
            }
            let id = self.graph_type_drop_admission(&base, path, existing)?;
            replaced = Some((id, summary));
        }
        let raw = next_id(base.high_water.graph_type, "graph type")?;
        let id = LowerGraphTypeId::new(raw).map_err(Error::from_catalog_invariant)?;
        let runtime = Arc::new(definition.into_runtime(path.object())?);
        let mut transaction =
            CatalogTransaction::new(&base.catalog).map_err(Error::from_catalog_invariant)?;
        if let Some((dropped, _)) = &replaced {
            transaction.remove(CatalogObjectId::GraphType(*dropped));
        }
        let descriptor = LowerDescriptor::graph_type(
            id,
            path.object.0.clone(),
            schema,
            transaction.generation(),
            CreationMetadata::new(transaction.generation(), None),
        )
        .map_err(Error::from_catalog_invariant)?;
        transaction
            .insert(descriptor)
            .map_err(Error::from_catalog_invariant)?;
        self.inner.after_descriptor_staging()?;
        let catalog = transaction.build().map_err(Error::from_catalog_invariant)?;
        let mut graph_types = base.graph_types.clone();
        if let Some((dropped, _)) = &replaced {
            graph_types.remove(dropped);
        }
        graph_types.insert(id, runtime);
        let mut high_water = base.high_water;
        high_water.graph_type = raw;
        let next = Arc::new(DatabaseState {
            catalog,
            graphs: base.graphs.clone(),
            graph_types,
            high_water,
        });
        let created = created_summary(&next, CatalogObjectId::GraphType(id), graph_type_summary)?;
        self.inner.before_publication()?;
        self.inner.state.store(next);
        match replaced {
            Some((_, dropped)) => Ok(CreateOutcome::Replaced { dropped, created }),
            None => Ok(CreateOutcome::Created(created)),
        }
    }

    /// Create an open graph or a graph constrained by a same-schema graph type.
    ///
    /// Under [`CreatePolicy::OrReplace`] an existing graph at the path is
    /// dropped with the same admission as [`Catalog::drop_graph`] and the new
    /// graph, with a fresh identity, is published in the same state swap. A
    /// failure at any point publishes nothing, so the old graph stays
    /// registered and its selected sessions stay valid.
    pub fn create_graph(
        &self,
        path: &ObjectPath,
        graph_type: Option<&ObjectPath>,
        policy: CreatePolicy,
    ) -> Result<CreateOutcome<GraphDescriptor>> {
        let _writer = self.inner.lock_lifecycle_writer();
        let base = self.inner.state.load_full();
        let schema = require_schema(&base, &path.schema_path())?;
        let mut replaced = None;
        let mut replaced_instance = None;
        if let Some(existing) = base.catalog.schema_object(schema, &path.object.0) {
            if existing.kind() != CatalogObjectKind::Graph {
                return Err(Error::wrong_kind(path, "graph", existing.kind()));
            }
            let summary = graph_summary(&base, existing)?;
            if policy != CreatePolicy::OrReplace {
                return duplicate_outcome(policy, summary, path, "graph");
            }
            let (id, instance) = self.drop_admission(&base, existing)?;
            replaced = Some((id, summary));
            replaced_instance = Some(instance);
        }
        // Lock order for the replaced graph is the drop order: writer mutex,
        // then its lifecycle write lease, then its graph state. The lease is
        // held until the replacing state is stored.
        let _lifecycle = replaced_instance
            .as_ref()
            .map(|instance| instance.lifecycle.write());
        if let (Some((id, _)), Some(instance)) = (&replaced, &replaced_instance) {
            self.check_droppable(*id, instance, path)?;
        }
        let (type_id, runtime) = resolve_binding(&base, schema, graph_type)?;
        let raw = next_id(base.high_water.graph, "graph")?;
        let id = LowerGraphId::new(raw).map_err(Error::from_catalog_invariant)?;
        let mut transaction =
            CatalogTransaction::new(&base.catalog).map_err(Error::from_catalog_invariant)?;
        let mut graphs = base.graphs.clone();
        if let Some((dropped, _)) = &replaced {
            transaction.remove(CatalogObjectId::Graph(*dropped));
            graphs.remove(dropped);
        }
        let descriptor = LowerDescriptor::graph(
            id,
            path.object.0.clone(),
            schema,
            transaction.generation(),
            CreationMetadata::new(transaction.generation(), None),
            type_id,
        )
        .map_err(Error::from_catalog_invariant)?;
        transaction
            .insert(descriptor)
            .map_err(Error::from_catalog_invariant)?;
        self.inner.after_descriptor_staging()?;
        let builder = SharedGraph::builder(core_graph_id(id));
        let graph = match runtime {
            Some(definition) => builder.bound_to((*definition).clone()),
            None => Ok(builder),
        }
        .and_then(selene_graph::SharedGraphBuilder::build)
        .map_err(Error::invalid_graph_type_source)?;
        let catalog = transaction.build().map_err(Error::from_catalog_invariant)?;
        graphs.insert(id, Arc::new(GraphInstance::new(graph)));
        let mut high_water = base.high_water;
        high_water.graph = raw;
        let next = Arc::new(DatabaseState {
            catalog,
            graphs,
            graph_types: base.graph_types.clone(),
            high_water,
        });
        let created = created_summary(&next, CatalogObjectId::Graph(id), graph_summary)?;
        self.inner.after_graph_construction()?;
        self.inner.before_publication()?;
        self.inner.state.store(next);
        match replaced {
            Some((dropped, summary)) => {
                self.inner.procedures.forget_graph(core_graph_id(dropped));
                Ok(CreateOutcome::Replaced {
                    dropped: summary,
                    created,
                })
            }
            None => Ok(CreateOutcome::Created(created)),
        }
    }

    /// Drop an empty graph under RESTRICT semantics.
    pub fn drop_graph(
        &self,
        path: &ObjectPath,
        policy: DropPolicy,
    ) -> Result<DropOutcome<GraphDescriptor>> {
        let _writer = self.inner.lock_lifecycle_writer();
        let base = self.inner.state.load_full();
        let Some(descriptor) = find_object(&base, path)? else {
            return missing_outcome(policy, path, "graph");
        };
        if descriptor.kind() != CatalogObjectKind::Graph {
            return Err(Error::wrong_kind(path, "graph", descriptor.kind()));
        }
        let (id, instance) = self.drop_admission(&base, descriptor)?;
        let _lifecycle = instance.lifecycle.write();
        self.check_droppable(id, &instance, path)?;
        let summary = graph_summary(&base, descriptor)?;
        let mut transaction =
            CatalogTransaction::new(&base.catalog).map_err(Error::from_catalog_invariant)?;
        transaction.remove(CatalogObjectId::Graph(id));
        self.inner.after_descriptor_staging()?;
        let catalog = transaction.build().map_err(Error::from_catalog_invariant)?;
        let mut graphs = base.graphs.clone();
        graphs.remove(&id);
        let next = Arc::new(DatabaseState {
            catalog,
            graphs,
            graph_types: base.graph_types.clone(),
            high_water: base.high_water,
        });
        self.inner.state.store(next);
        self.inner.procedures.forget_graph(core_graph_id(id));
        Ok(DropOutcome::Dropped(summary))
    }

    /// Drop an unreferenced graph type under RESTRICT semantics.
    pub fn drop_graph_type(
        &self,
        path: &ObjectPath,
        policy: DropPolicy,
    ) -> Result<DropOutcome<GraphTypeDescriptor>> {
        let _writer = self.inner.lock_lifecycle_writer();
        let base = self.inner.state.load_full();
        let Some(descriptor) = find_object(&base, path)? else {
            return missing_outcome(policy, path, "graph type");
        };
        if descriptor.kind() != CatalogObjectKind::GraphType {
            return Err(Error::wrong_kind(path, "graph type", descriptor.kind()));
        }
        let id = self.graph_type_drop_admission(&base, path, descriptor)?;
        let summary = graph_type_summary(&base, descriptor)?;
        let mut transaction =
            CatalogTransaction::new(&base.catalog).map_err(Error::from_catalog_invariant)?;
        transaction.remove(CatalogObjectId::GraphType(id));
        self.inner.after_descriptor_staging()?;
        let catalog = transaction.build().map_err(Error::from_catalog_invariant)?;
        let mut graph_types = base.graph_types.clone();
        graph_types.remove(&id);
        let next = Arc::new(DatabaseState {
            catalog,
            graphs: base.graphs.clone(),
            graph_types,
            high_water: base.high_water,
        });
        self.inner.state.store(next);
        Ok(DropOutcome::Dropped(summary))
    }

    /// Apply the dependency check shared by DROP and OR REPLACE.
    fn graph_type_drop_admission(
        &self,
        base: &DatabaseState,
        path: &ObjectPath,
        descriptor: &LowerDescriptor,
    ) -> Result<LowerGraphTypeId> {
        let CatalogObjectId::GraphType(id) = descriptor.id() else {
            unreachable!("kind and typed ID agree")
        };
        let references = base
            .catalog
            .descriptors()
            .filter(|candidate| {
                matches!(candidate.payload(), CatalogPayload::Graph { graph_type: Some(target) } if *target == id)
            })
            .count();
        if references != 0 {
            return Err(Error::referenced_graph_type(path, references));
        }
        Ok(id)
    }

    /// Drop an empty schema under RESTRICT semantics.
    pub fn drop_schema(
        &self,
        path: &SchemaPath,
        policy: DropPolicy,
    ) -> Result<DropOutcome<SchemaDescriptor>> {
        let _writer = self.inner.lock_lifecycle_writer();
        let base = self.inner.state.load_full();
        ensure_catalog(&base, path.catalog())?;
        let Some(descriptor) = base.catalog.schema(&path.schema.0) else {
            return missing_outcome(policy, path, "schema");
        };
        let CatalogObjectId::Schema(id) = descriptor.id() else {
            unreachable!("schema dictionary contains schemas")
        };
        let children = base.catalog.schema_objects(id).map_or(0, Iterator::count);
        if children != 0 {
            return Err(Error::nonempty_schema(path, children));
        }
        let summary = schema_summary(&base, descriptor)?;
        let mut transaction =
            CatalogTransaction::new(&base.catalog).map_err(Error::from_catalog_invariant)?;
        transaction.remove(CatalogObjectId::Schema(id));
        self.inner.after_descriptor_staging()?;
        let catalog = transaction.build().map_err(Error::from_catalog_invariant)?;
        let next = Arc::new(DatabaseState {
            catalog,
            graphs: base.graphs.clone(),
            graph_types: base.graph_types.clone(),
            high_water: base.high_water,
        });
        self.inner.state.store(next);
        Ok(DropOutcome::Dropped(summary))
    }

    /// Locate the registered instance before taking its lifecycle write lease.
    fn drop_admission(
        &self,
        base: &DatabaseState,
        descriptor: &LowerDescriptor,
    ) -> Result<(LowerGraphId, Arc<GraphInstance>)> {
        let CatalogObjectId::Graph(id) = descriptor.id() else {
            unreachable!("kind and typed ID agree")
        };
        let instance = base.graphs.get(&id).cloned().ok_or_else(|| {
            Error::catalog_invariant("graph descriptor has no registered runtime instance")
        })?;
        #[cfg(test)]
        self.inner.observe_blocked_drop(&instance);
        Ok((id, instance))
    }

    /// Second half of graph-drop admission, under the instance's lifecycle
    /// write lease: recheck registration, then apply RESTRICT to the contents
    /// left by every request that completed before the lease was granted.
    fn check_droppable(
        &self,
        id: LowerGraphId,
        instance: &Arc<GraphInstance>,
        path: &ObjectPath,
    ) -> Result<()> {
        let current = self.inner.state.load_full();
        if current
            .graphs
            .get(&id)
            .is_none_or(|registered| !Arc::ptr_eq(registered, instance))
        {
            return Err(Error::stale_graph(path));
        }
        let graph = instance.graph.read();
        let nodes = graph.node_count();
        let edges = graph.edge_count();
        drop(graph);
        if nodes != 0 || edges != 0 {
            return Err(Error::nonempty_graph(path, nodes, edges));
        }
        Ok(())
    }
}

fn duplicate_outcome<T>(
    policy: CreatePolicy,
    value: T,
    path: &impl std::fmt::Display,
    kind: &'static str,
) -> Result<CreateOutcome<T>> {
    match policy {
        CreatePolicy::Strict => Err(Error::already_exists(path)),
        CreatePolicy::IfNotExists => Ok(CreateOutcome::AlreadyExists(value)),
        // Graph and graph-type replacement are handled before this point;
        // schema replacement is rejected up front.
        CreatePolicy::OrReplace => Err(Error::unsupported_create_policy(kind)),
    }
}

/// Reject [`CreatePolicy::OrReplace`] for kinds that do not implement it,
/// before any lock is taken and regardless of whether the object exists.
fn reject_replace(policy: CreatePolicy, kind: &'static str) -> Result<()> {
    if policy == CreatePolicy::OrReplace {
        return Err(Error::unsupported_create_policy(kind));
    }
    Ok(())
}

fn missing_outcome<T>(
    policy: DropPolicy,
    path: &impl std::fmt::Display,
    kind: &'static str,
) -> Result<DropOutcome<T>> {
    match policy {
        DropPolicy::Strict => Err(Error::not_found(path, kind)),
        DropPolicy::IfExists => Ok(DropOutcome::NotFound),
    }
}

fn created_summary<T>(
    state: &DatabaseState,
    id: CatalogObjectId,
    summarize: impl FnOnce(&DatabaseState, &LowerDescriptor) -> Result<T>,
) -> Result<T> {
    let descriptor = state
        .catalog
        .descriptor(id)
        .ok_or_else(|| Error::catalog_invariant("created descriptor is missing"))?;
    summarize(state, descriptor)
}

pub(crate) const fn core_graph_id(id: LowerGraphId) -> CoreGraphId {
    CoreGraphId::new(id.get())
}

impl DatabaseInner {
    #[cfg(test)]
    fn observe_blocked_drop(&self, instance: &GraphInstance) {
        if let Some(observer) = self.drop_blocked.lock().take() {
            assert!(
                instance.lifecycle.try_write().is_none(),
                "drop observer requires an active request lease"
            );
            observer.send(()).expect("drop observer is listening");
        }
    }

    fn after_descriptor_staging(&self) -> Result<()> {
        #[cfg(test)]
        if *self.failure.lock() == Some(FailurePoint::AfterDescriptorStaging) {
            self.failure.lock().take();
            return Err(Error::injected_failure("after descriptor staging"));
        }
        Ok(())
    }

    fn after_graph_construction(&self) -> Result<()> {
        #[cfg(test)]
        if *self.failure.lock() == Some(FailurePoint::AfterGraphConstruction) {
            self.failure.lock().take();
            return Err(Error::injected_failure("after graph construction"));
        }
        Ok(())
    }

    fn before_publication(&self) -> Result<()> {
        #[cfg(test)]
        if *self.failure.lock() == Some(FailurePoint::BeforePublication) {
            self.failure.lock().take();
            return Err(Error::injected_failure("before publication"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailurePoint {
    AfterDescriptorStaging,
    AfterGraphConstruction,
    BeforePublication,
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
