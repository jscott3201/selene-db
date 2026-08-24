//! Facade-owned embedded authorization and principal contracts.

use std::{error::Error as StdError, fmt, sync::Arc};

use selene_core::{DbString, db_string::MAX_DB_STRING_BYTES};

use crate::{Error, GraphDescriptor, ObjectPath, Result, SchemaDescriptor, SchemaPath};

macro_rules! text_identifier {
    ($name:ident, $kind:literal, $error:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(transparent)]
        pub struct $name(DbString);

        impl $name {
            /// Construct a non-empty database-string identifier.
            ///
            /// # Errors
            ///
            /// Returns a facade diagnostic when `value` is empty or exceeds
            /// the IL013 database-string byte limit.
            pub fn new(value: &str) -> Result<Self> {
                validate_identifier_length($kind, value.len())?;
                DbString::try_from(value).map(Self).map_err(Error::$error)
            }

            /// Return the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }
    };
}

text_identifier!(
    AuthorizationId,
    "authorization",
    invalid_authorization_id_source,
    "Opaque authorization identifier with GQL `STRING` semantics."
);
text_identifier!(
    PrincipalId,
    "principal",
    invalid_principal_id_source,
    "Opaque principal identifier with GQL `STRING` semantics."
);

fn validate_identifier_length(kind: &'static str, byte_len: usize) -> Result<()> {
    if byte_len == 0 {
        return Err(Error::empty_identity(kind));
    }
    if byte_len > MAX_DB_STRING_BYTES {
        return Err(Error::identity_too_long(kind));
    }
    Ok(())
}

/// Facade principal resolved by an embedder-owned provider.
///
/// The string principal ID and optional opaque audit bytes have separate
/// contracts. Audit bytes are forwarded to graph commits but are not the
/// `SESSION_USER` value.
#[derive(Clone)]
pub struct Principal {
    id: PrincipalId,
    audit_bytes: Option<Arc<[u8]>>,
    home_schema: Option<SchemaPath>,
    home_graph: Option<ObjectPath>,
}

impl Principal {
    /// Construct a principal without audit bytes or declared homes.
    #[must_use]
    pub const fn new(id: PrincipalId) -> Self {
        Self {
            id,
            audit_bytes: None,
            home_schema: None,
            home_graph: None,
        }
    }

    /// Attach opaque audit bytes forwarded to commits.
    #[must_use]
    pub fn with_audit_bytes(mut self, audit_bytes: impl Into<Arc<[u8]>>) -> Self {
        self.audit_bytes = Some(audit_bytes.into());
        self
    }

    /// Declare the principal's home schema path.
    #[must_use]
    pub fn with_home_schema(mut self, path: SchemaPath) -> Self {
        self.home_schema = Some(path);
        self
    }

    /// Declare the principal's home graph path.
    ///
    /// Session initialization rejects this declaration unless a coherent home
    /// schema is also declared.
    #[must_use]
    pub fn with_home_graph(mut self, path: ObjectPath) -> Self {
        self.home_graph = Some(path);
        self
    }

    /// Return the principal's string identity.
    #[must_use]
    pub const fn id(&self) -> &PrincipalId {
        &self.id
    }

    /// Return opaque audit bytes, when configured.
    #[must_use]
    pub fn audit_bytes(&self) -> Option<&[u8]> {
        self.audit_bytes.as_deref()
    }

    /// Return the declared home schema path, when present.
    #[must_use]
    pub const fn home_schema(&self) -> Option<&SchemaPath> {
        self.home_schema.as_ref()
    }

    /// Return the declared home graph path, when present.
    #[must_use]
    pub const fn home_graph(&self) -> Option<&ObjectPath> {
        self.home_graph.as_ref()
    }

    pub(crate) fn audit_bytes_arc(&self) -> Option<Arc<[u8]>> {
        self.audit_bytes.clone()
    }
}

/// Deliberate failure returned by an embedder authorization hook.
///
/// The value carries no caller text, so facade diagnostics cannot accidentally
/// disclose credentials, identifiers, or tokens supplied to the hook.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct AuthHookError {
    _private: (),
}

impl AuthHookError {
    /// Construct a deterministic hook failure.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Display for AuthHookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("embedded authorization hook failed")
    }
}

impl StdError for AuthHookError {}

/// Embedder-owned mapping from an authorization ID to a principal.
pub trait PrincipalProvider: Send + Sync {
    /// Resolve one authorization ID.
    ///
    /// `Ok(None)` means the supplied authorization ID has no principal.
    fn resolve(
        &self,
        authorization_id: &AuthorizationId,
    ) -> std::result::Result<Option<Principal>, AuthHookError>;
}

/// Deterministic local provider with no principal directory.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoPrincipalProvider;

impl PrincipalProvider for NoPrincipalProvider {
    fn resolve(
        &self,
        _authorization_id: &AuthorizationId,
    ) -> std::result::Result<Option<Principal>, AuthHookError> {
        Ok(None)
    }
}

/// Authorization decision returned by an embedded policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AuthorizationDecision {
    /// Permit session creation.
    Allow,
    /// Refuse session creation.
    Deny,
}

/// Immutable facade-only input to an embedded authorization policy.
pub struct AuthorizationRequest<'a> {
    authorization_id: Option<&'a AuthorizationId>,
    principal: Option<&'a Principal>,
    home_schema: Option<&'a SchemaDescriptor>,
    home_graph: Option<&'a GraphDescriptor>,
    current_schema: &'a SchemaDescriptor,
    current_graph: &'a GraphDescriptor,
}

impl<'a> AuthorizationRequest<'a> {
    pub(crate) const fn new(
        authorization_id: Option<&'a AuthorizationId>,
        principal: Option<&'a Principal>,
        home_schema: Option<&'a SchemaDescriptor>,
        home_graph: Option<&'a GraphDescriptor>,
        current_schema: &'a SchemaDescriptor,
        current_graph: &'a GraphDescriptor,
    ) -> Self {
        Self {
            authorization_id,
            principal,
            home_schema,
            home_graph,
            current_schema,
            current_graph,
        }
    }

    /// Return the authorization ID supplied for this session.
    #[must_use]
    pub const fn authorization_id(&self) -> Option<&AuthorizationId> {
        self.authorization_id
    }

    /// Return the resolved principal.
    #[must_use]
    pub const fn principal(&self) -> Option<&Principal> {
        self.principal
    }

    /// Return the resolved home schema descriptor.
    #[must_use]
    pub const fn home_schema(&self) -> Option<&SchemaDescriptor> {
        self.home_schema
    }

    /// Return the resolved home graph descriptor.
    #[must_use]
    pub const fn home_graph(&self) -> Option<&GraphDescriptor> {
        self.home_graph
    }

    /// Return the selected current schema descriptor.
    #[must_use]
    pub const fn current_schema(&self) -> &SchemaDescriptor {
        self.current_schema
    }

    /// Return the selected current graph descriptor.
    #[must_use]
    pub const fn current_graph(&self) -> &GraphDescriptor {
        self.current_graph
    }
}

/// Embedded policy applied to copied facade session descriptors.
pub trait AuthorizationPolicy: Send + Sync {
    /// Decide whether session creation is permitted.
    fn authorize(
        &self,
        request: &AuthorizationRequest<'_>,
    ) -> std::result::Result<AuthorizationDecision, AuthHookError>;
}

/// Deterministic local policy that permits every session.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAllAuthorizationPolicy;

impl AuthorizationPolicy for AllowAllAuthorizationPolicy {
    fn authorize(
        &self,
        _request: &AuthorizationRequest<'_>,
    ) -> std::result::Result<AuthorizationDecision, AuthHookError> {
        Ok(AuthorizationDecision::Allow)
    }
}

/// Per-session authorization and policy configuration.
#[derive(Clone)]
pub struct SessionOptions {
    pub(crate) authorization_id: Option<AuthorizationId>,
    pub(crate) principal_provider: Arc<dyn PrincipalProvider>,
    pub(crate) authorization_policy: Arc<dyn AuthorizationPolicy>,
}

impl SessionOptions {
    /// Construct anonymous options with the deterministic local hooks.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply the authorization ID resolved for this session.
    #[must_use]
    pub fn with_authorization_id(mut self, authorization_id: AuthorizationId) -> Self {
        self.authorization_id = Some(authorization_id);
        self
    }

    /// Replace the principal provider used when an authorization ID is present.
    #[must_use]
    pub fn with_principal_provider(mut self, provider: Arc<dyn PrincipalProvider>) -> Self {
        self.principal_provider = provider;
        self
    }

    /// Replace the policy applied after catalog references are copied.
    #[must_use]
    pub fn with_authorization_policy(mut self, policy: Arc<dyn AuthorizationPolicy>) -> Self {
        self.authorization_policy = policy;
        self
    }

    /// Return the configured authorization ID.
    #[must_use]
    pub const fn authorization_id(&self) -> Option<&AuthorizationId> {
        self.authorization_id.as_ref()
    }
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            authorization_id: None,
            principal_provider: Arc::new(NoPrincipalProvider),
            authorization_policy: Arc::new(AllowAllAuthorizationPolicy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_length_guard_uses_the_full_il013_limit() {
        assert!(validate_identifier_length("authorization", MAX_DB_STRING_BYTES).is_ok());
        assert_eq!(
            validate_identifier_length("authorization", MAX_DB_STRING_BYTES + 1)
                .unwrap_err()
                .kind(),
            crate::ErrorKind::InvalidAuthorizationId
        );
        assert_eq!(
            validate_identifier_length("principal", MAX_DB_STRING_BYTES + 1)
                .unwrap_err()
                .kind(),
            crate::ErrorKind::InvalidPrincipalId
        );
    }
}
