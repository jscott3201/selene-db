//! Facade session state and immutable dependency inspection.

use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use selene_core::DbString;
use selene_profile::{current_profile_identity, current_session_defaults};

use crate::{
    AuthorizationId, CatalogGeneration, Error, GeneralParameter, GraphDescriptor, GraphId,
    Principal, RequestContext, Result, SchemaDescriptor, SchemaId, Transaction, TransactionState,
    transaction::DetachedTransaction,
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

/// Snapshot inspection of session parameter state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionParameters {
    count: u64,
}

impl SessionParameters {
    fn from_count(count: usize) -> Self {
        Self {
            count: count as u64,
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
    /// One request context is associated with the session.
    Active,
}

/// Session transaction-slot inspection state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransactionSlotState {
    /// No transaction is active.
    Vacant,
    /// Requests may execute and stage work.
    Active,
    /// A statement failed and detached work was discarded.
    Failed,
    /// Commit validation/publication is in progress.
    Committing,
    /// Detached work was discarded.
    RolledBack,
    /// The complete successor state was published and acknowledged.
    Committed,
    /// Publication completed but acknowledgement was uncertain.
    Indeterminate,
}

impl From<TransactionState> for TransactionSlotState {
    fn from(state: TransactionState) -> Self {
        match state {
            TransactionState::Active => Self::Active,
            TransactionState::Failed => Self::Failed,
            TransactionState::Committing => Self::Committing,
            TransactionState::RolledBack => Self::RolledBack,
            TransactionState::Committed => Self::Committed,
            TransactionState::Indeterminate => Self::Indeterminate,
        }
    }
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

#[derive(Default)]
struct SessionState {
    parameters: BTreeMap<DbString, GeneralParameter>,
    active_request: Option<Arc<RequestContext>>,
    transaction: Option<DetachedTransaction>,
}

pub(crate) struct ActiveRequestGuard<'a> {
    state: &'a RefCell<SessionState>,
    context: Arc<RequestContext>,
}

pub(crate) struct TransactionCheckout<'a> {
    state: &'a RefCell<SessionState>,
    transaction: Option<DetachedTransaction>,
}

impl TransactionCheckout<'_> {
    pub(crate) fn as_ref(&self) -> Option<&DetachedTransaction> {
        self.transaction.as_ref()
    }

    pub(crate) fn as_mut(&mut self) -> Option<&mut DetachedTransaction> {
        self.transaction.as_mut()
    }

    pub(crate) fn replace(
        &mut self,
        transaction: DetachedTransaction,
    ) -> Option<DetachedTransaction> {
        self.transaction.replace(transaction)
    }
}

impl Drop for TransactionCheckout<'_> {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(transaction) = self.transaction.as_mut()
            && matches!(
                transaction.descriptor().state(),
                TransactionState::Active | TransactionState::Committing
            )
        {
            transaction.abandon_on_unwind();
        }
        self.state.borrow_mut().transaction = self.transaction.take();
    }
}

impl Drop for ActiveRequestGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        if std::thread::panicking()
            && let Some(transaction) = state.transaction.as_mut()
            && matches!(
                transaction.descriptor().state(),
                TransactionState::Active | TransactionState::Committing
            )
        {
            transaction.abandon_on_unwind();
        }
        if state
            .active_request
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &self.context))
        {
            state.active_request = None;
        }
    }
}

/// Immutable creation dependencies and controlled state for one session.
///
/// The context owns facade descriptors, typed parameters, and at most one
/// request context. It retains no catalog read snapshot, runtime graph
/// allocation, lifecycle lease, or lower engine handle.
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
    state: RefCell<SessionState>,
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
            state: RefCell::new(SessionState::default()),
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

    /// Return a count snapshot of the current session parameter dictionary.
    #[must_use]
    pub fn parameters(&self) -> SessionParameters {
        SessionParameters::from_count(self.state.borrow().parameters.len())
    }

    /// Return whether a request context is currently associated with the session.
    #[must_use]
    pub fn request_slot(&self) -> RequestSlotState {
        if self.state.borrow().active_request.is_some() {
            RequestSlotState::Active
        } else {
            RequestSlotState::Vacant
        }
    }

    /// Clone the actual active request context, when the slot is occupied.
    #[must_use]
    pub fn current_request(&self) -> Option<Arc<RequestContext>> {
        self.state.borrow().active_request.as_ref().map(Arc::clone)
    }

    /// Return the precise transaction slot state.
    #[must_use]
    pub fn transaction_slot(&self) -> TransactionSlotState {
        self.state
            .borrow()
            .transaction
            .as_ref()
            .map_or(TransactionSlotState::Vacant, |transaction| {
                transaction.descriptor().state().into()
            })
    }

    /// Clone the immutable transaction inspection descriptor, when present.
    #[must_use]
    pub fn transaction(&self) -> Option<Transaction> {
        self.state
            .borrow()
            .transaction
            .as_ref()
            .map(|transaction| transaction.descriptor().clone())
    }

    /// Return the session termination state.
    #[must_use]
    pub const fn termination(&self) -> SessionTerminationState {
        self.termination
    }

    pub(crate) fn parameter_snapshot(&self) -> BTreeMap<DbString, GeneralParameter> {
        self.state.borrow().parameters.clone()
    }

    pub(crate) fn set_parameter(
        &self,
        name: DbString,
        parameter: GeneralParameter,
    ) -> Option<GeneralParameter> {
        self.state.borrow_mut().parameters.insert(name, parameter)
    }

    pub(crate) fn remove_parameter(&self, name: &str) -> Option<GeneralParameter> {
        self.state.borrow_mut().parameters.remove(name)
    }

    pub(crate) fn checkout_transaction(&self) -> TransactionCheckout<'_> {
        let transaction = self.state.borrow_mut().transaction.take();
        TransactionCheckout {
            state: &self.state,
            transaction,
        }
    }

    pub(crate) fn activate_request(
        &self,
        context: Arc<RequestContext>,
    ) -> Result<ActiveRequestGuard<'_>> {
        let mut state = self.state.borrow_mut();
        if state.active_request.is_some() {
            return Err(Error::request_already_active());
        }
        state.active_request = Some(Arc::clone(&context));
        drop(state);
        Ok(ActiveRequestGuard {
            state: &self.state,
            context,
        })
    }
}
