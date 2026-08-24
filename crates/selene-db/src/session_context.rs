//! Immutable facade session context and dependency inspection.

use selene_profile::{current_profile_identity, current_session_defaults};

use crate::{
    AuthorizationId, CatalogGeneration, GraphDescriptor, GraphId, Principal, SchemaDescriptor,
    SchemaId,
};

/// Facade-owned copy of the generated profile identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProfileIdentity {
    profile_id: String,
    source_format_version: u32,
    generator_version: u32,
    canonical_hash: String,
}

impl ProfileIdentity {
    fn current() -> Self {
        let identity = current_profile_identity();
        Self {
            profile_id: identity.profile_id().to_owned(),
            source_format_version: identity.source_format_version(),
            generator_version: identity.generator_version(),
            canonical_hash: identity.canonical_hash().to_owned(),
        }
    }

    /// Return the stable target-profile identifier.
    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// Return the incompatible profile source-format version.
    #[must_use]
    pub const fn source_format_version(&self) -> u32 {
        self.source_format_version
    }

    /// Return the deterministic generator-contract version.
    #[must_use]
    pub const fn generator_version(&self) -> u32 {
        self.generator_version
    }

    /// Return the canonical profile BLAKE3 hash.
    #[must_use]
    pub fn canonical_hash(&self) -> &str {
        &self.canonical_hash
    }
}

/// Fixed session displacement from UTC.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TimeZoneDisplacement {
    seconds: i32,
}

impl TimeZoneDisplacement {
    fn from_profile() -> Self {
        Self {
            seconds: current_session_defaults().time_zone().seconds(),
        }
    }

    /// Return the signed UTC displacement in seconds.
    #[must_use]
    pub const fn seconds(self) -> i32 {
        self.seconds
    }
}

/// Immutable inspection of session parameter state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionParameters {
    count: u64,
}

impl SessionParameters {
    fn from_profile() -> Self {
        Self {
            count: current_session_defaults().initial_parameter_count(),
        }
    }

    /// Return the number of session parameters.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.count
    }

    /// Return whether the session parameter dictionary is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Session request-slot inspection state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RequestSlotState {
    /// No request is active.
    Vacant,
}

/// Session transaction-slot inspection state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransactionSlotState {
    /// No transaction is active.
    Vacant,
}

/// Session termination inspection state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SessionTerminationState {
    /// The session accepts requests.
    Active,
}

/// Immutable dependencies captured when a session is created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDependencySummary {
    current_schema: SchemaId,
    current_graph: GraphId,
    home_schema: Option<SchemaId>,
    home_graph: Option<GraphId>,
    profile: ProfileIdentity,
}

impl SessionDependencySummary {
    /// Return the stable current schema identity.
    #[must_use]
    pub const fn current_schema(&self) -> SchemaId {
        self.current_schema
    }

    /// Return the stable current graph identity.
    #[must_use]
    pub const fn current_graph(&self) -> GraphId {
        self.current_graph
    }

    /// Return the stable home schema identity, when configured.
    #[must_use]
    pub const fn home_schema(&self) -> Option<SchemaId> {
        self.home_schema
    }

    /// Return the stable home graph identity, when configured.
    #[must_use]
    pub const fn home_graph(&self) -> Option<GraphId> {
        self.home_graph
    }

    /// Return the generated profile identity used by this session.
    #[must_use]
    pub const fn profile_identity(&self) -> &ProfileIdentity {
        &self.profile
    }
}

pub(crate) struct SessionContextParts {
    pub(crate) authorization_id: Option<AuthorizationId>,
    pub(crate) principal: Option<Principal>,
    pub(crate) home_schema: Option<SchemaDescriptor>,
    pub(crate) home_graph: Option<GraphDescriptor>,
    pub(crate) current_schema: SchemaDescriptor,
    pub(crate) current_graph: GraphDescriptor,
    pub(crate) catalog_generation: CatalogGeneration,
}

/// Immutable creation snapshot and inactive state slots for one session.
///
/// The context owns only facade descriptors and scalar metadata. It retains no
/// catalog read snapshot, runtime graph allocation, lifecycle lease, or lower
/// engine handle.
pub struct SessionContext {
    authorization_id: Option<AuthorizationId>,
    principal: Option<Principal>,
    home_schema: Option<SchemaDescriptor>,
    home_graph: Option<GraphDescriptor>,
    current_schema: SchemaDescriptor,
    current_graph: GraphDescriptor,
    catalog_generation: CatalogGeneration,
    dependencies: SessionDependencySummary,
    time_zone: TimeZoneDisplacement,
    parameters: SessionParameters,
    request_slot: RequestSlotState,
    transaction_slot: TransactionSlotState,
    termination: SessionTerminationState,
}

impl SessionContext {
    pub(crate) fn new(parts: SessionContextParts) -> Self {
        let profile = ProfileIdentity::current();
        let dependencies = SessionDependencySummary {
            current_schema: parts.current_schema.id,
            current_graph: parts.current_graph.id,
            home_schema: parts.home_schema.as_ref().map(|schema| schema.id),
            home_graph: parts.home_graph.as_ref().map(|graph| graph.id),
            profile,
        };
        Self {
            authorization_id: parts.authorization_id,
            principal: parts.principal,
            home_schema: parts.home_schema,
            home_graph: parts.home_graph,
            current_schema: parts.current_schema,
            current_graph: parts.current_graph,
            catalog_generation: parts.catalog_generation,
            dependencies,
            time_zone: TimeZoneDisplacement::from_profile(),
            parameters: SessionParameters::from_profile(),
            request_slot: RequestSlotState::Vacant,
            transaction_slot: TransactionSlotState::Vacant,
            termination: SessionTerminationState::Active,
        }
    }

    /// Return the authorization ID supplied at creation.
    #[must_use]
    pub const fn authorization_id(&self) -> Option<&AuthorizationId> {
        self.authorization_id.as_ref()
    }

    /// Return the resolved principal.
    #[must_use]
    pub const fn principal(&self) -> Option<&Principal> {
        self.principal.as_ref()
    }

    /// Return the copied home schema descriptor.
    #[must_use]
    pub const fn home_schema(&self) -> Option<&SchemaDescriptor> {
        self.home_schema.as_ref()
    }

    /// Return the copied home graph descriptor.
    #[must_use]
    pub const fn home_graph(&self) -> Option<&GraphDescriptor> {
        self.home_graph.as_ref()
    }

    /// Return the copied current schema descriptor.
    #[must_use]
    pub const fn current_schema(&self) -> &SchemaDescriptor {
        &self.current_schema
    }

    /// Return the copied current graph descriptor.
    #[must_use]
    pub const fn current_graph(&self) -> &GraphDescriptor {
        &self.current_graph
    }

    /// Return the catalog generation observed at creation.
    #[must_use]
    pub const fn catalog_generation(&self) -> CatalogGeneration {
        self.catalog_generation
    }

    /// Return the immutable catalog/profile dependency summary.
    #[must_use]
    pub const fn dependencies(&self) -> &SessionDependencySummary {
        &self.dependencies
    }

    /// Return the copied generated profile identity.
    #[must_use]
    pub const fn profile_identity(&self) -> &ProfileIdentity {
        self.dependencies.profile_identity()
    }

    /// Return the fixed session time-zone displacement.
    #[must_use]
    pub const fn time_zone(&self) -> TimeZoneDisplacement {
        self.time_zone
    }

    /// Return immutable session parameter inspection.
    #[must_use]
    pub const fn parameters(&self) -> &SessionParameters {
        &self.parameters
    }

    /// Return the inactive request slot state.
    #[must_use]
    pub const fn request_slot(&self) -> RequestSlotState {
        self.request_slot
    }

    /// Return the inactive transaction slot state.
    #[must_use]
    pub const fn transaction_slot(&self) -> TransactionSlotState {
        self.transaction_slot
    }

    /// Return the session termination state.
    #[must_use]
    pub const fn termination(&self) -> SessionTerminationState {
        self.termination
    }
}
