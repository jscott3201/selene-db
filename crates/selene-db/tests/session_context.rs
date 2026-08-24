//! Public session-context, principal-hook, and stable-reference behavior.

use std::{
    error::Error as _,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use selene_db::{
    AuthHookError, AuthorizationDecision, AuthorizationId, AuthorizationPolicy,
    AuthorizationRequest, Catalog, CreatePolicy, Database, DropPolicy, ErrorKind,
    GraphTypeDefinition, NodeTypeDefinition, ObjectPath, PathSegment, Principal, PrincipalId,
    PrincipalProvider, RequestSlotState, SchemaPath, SessionOptions, SessionTerminationState,
    TransactionSlotState,
};
use selene_profile::{
    PROFILE_FORMAT_VERSION, PROFILE_GENERATOR_VERSION, PROFILE_HASH, PROFILE_ID,
    current_session_defaults,
};

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).unwrap()
}

fn graph(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).unwrap()
}

fn fixture() -> (Database, ObjectPath) {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let current = graph("current", "main");
    catalog
        .create_schema(&schema("current"), CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&current, None, CreatePolicy::Strict)
        .unwrap();
    (database, current)
}

#[derive(Clone)]
enum ProviderResponse {
    Principal(Box<Principal>),
    Missing,
    Failure,
}

struct TestProvider {
    response: ProviderResponse,
    calls: Arc<AtomicUsize>,
    reenter: Option<(Catalog, SchemaPath)>,
}

impl TestProvider {
    fn principal(principal: Principal) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                response: ProviderResponse::Principal(Box::new(principal)),
                calls: Arc::clone(&calls),
                reenter: None,
            }),
            calls,
        )
    }

    fn with_response(response: ProviderResponse) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                response,
                calls: Arc::clone(&calls),
                reenter: None,
            }),
            calls,
        )
    }
}

impl PrincipalProvider for TestProvider {
    fn resolve(
        &self,
        _authorization_id: &AuthorizationId,
    ) -> std::result::Result<Option<Principal>, AuthHookError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some((catalog, path)) = &self.reenter {
            catalog
                .snapshot()
                .resolve_schema(path)
                .expect("provider can re-enter catalog reads");
        }
        match &self.response {
            ProviderResponse::Principal(principal) => Ok(Some((**principal).clone())),
            ProviderResponse::Missing => Ok(None),
            ProviderResponse::Failure => Err(AuthHookError::new()),
        }
    }
}

#[derive(Clone, Copy)]
enum PolicyResponse {
    Allow,
    Deny,
    Failure,
}

struct TestPolicy {
    response: PolicyResponse,
    calls: Arc<AtomicUsize>,
    reenter: Option<(Catalog, SchemaPath)>,
    expect_homes: bool,
}

impl TestPolicy {
    fn new(response: PolicyResponse) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                response,
                calls: Arc::clone(&calls),
                reenter: None,
                expect_homes: false,
            }),
            calls,
        )
    }
}

impl AuthorizationPolicy for TestPolicy {
    fn authorize(
        &self,
        request: &AuthorizationRequest<'_>,
    ) -> std::result::Result<AuthorizationDecision, AuthHookError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(
            request.authorization_id().map(AuthorizationId::as_str),
            Some("secret-auth")
        );
        assert_eq!(
            request.principal().map(|principal| principal.id().as_str()),
            Some("secret-principal")
        );
        assert_eq!(request.current_schema().path, schema("current"));
        assert_eq!(request.current_graph().path, graph("current", "main"));
        assert_eq!(request.home_schema().is_some(), self.expect_homes);
        assert_eq!(request.home_graph().is_some(), self.expect_homes);
        if let Some((catalog, path)) = &self.reenter {
            catalog
                .snapshot()
                .resolve_schema(path)
                .expect("policy can re-enter catalog reads");
        }
        match self.response {
            PolicyResponse::Allow => Ok(AuthorizationDecision::Allow),
            PolicyResponse::Deny => Ok(AuthorizationDecision::Deny),
            PolicyResponse::Failure => Err(AuthHookError::new()),
        }
    }
}

fn principal() -> Principal {
    Principal::new(PrincipalId::new("secret-principal").unwrap())
}

fn options_with_principal(principal: Principal) -> SessionOptions {
    let (provider, _) = TestProvider::principal(principal);
    SessionOptions::new()
        .with_authorization_id(AuthorizationId::new("secret-auth").unwrap())
        .with_principal_provider(provider)
}

#[test]
fn anonymous_context_copies_generated_defaults_and_dependencies() {
    let (database, current) = fixture();
    let snapshot = database.catalog().snapshot();
    let expected_graph = snapshot.resolve_graph(&current).unwrap();
    let expected_schema = snapshot.resolve_schema(&schema("current")).unwrap();
    let session = database.session(&current).unwrap();
    let context = session.context();

    assert!(context.authorization_id().is_none());
    assert!(context.principal().is_none());
    assert!(context.home_schema().is_none());
    assert!(context.home_graph().is_none());
    assert_eq!(context.current_schema(), &expected_schema);
    assert_eq!(context.current_graph(), &expected_graph);
    assert_eq!(context.catalog_generation(), snapshot.generation());
    assert_eq!(context.time_zone().seconds(), 0);
    assert_eq!(
        context.time_zone().seconds(),
        current_session_defaults().time_zone().seconds()
    );
    assert!(context.parameters().is_empty());
    assert_eq!(
        context.parameters().len(),
        current_session_defaults().initial_parameter_count()
    );
    assert_eq!(context.request_slot(), RequestSlotState::Vacant);
    assert_eq!(context.transaction_slot(), TransactionSlotState::Vacant);
    assert_eq!(context.termination(), SessionTerminationState::Active);

    let identity = context.profile_identity();
    assert_eq!(identity.profile_id(), PROFILE_ID);
    assert_eq!(identity.source_format_version(), PROFILE_FORMAT_VERSION);
    assert_eq!(identity.generator_version(), PROFILE_GENERATOR_VERSION);
    assert_eq!(identity.canonical_hash(), PROFILE_HASH);
    let dependencies = context.dependencies();
    assert_eq!(dependencies.current_schema(), expected_schema.id);
    assert_eq!(dependencies.current_graph(), expected_graph.id);
    assert_eq!(dependencies.home_schema(), None);
    assert_eq!(dependencies.home_graph(), None);
    assert_eq!(dependencies.profile_identity(), identity);
}

#[test]
fn anonymous_session_does_not_invoke_a_configured_provider() {
    let (database, current) = fixture();
    let (provider, calls) = TestProvider::with_response(ProviderResponse::Failure);
    let options = SessionOptions::new().with_principal_provider(provider);

    database.session_with_options(&current, options).unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn authenticated_allow_path_resolves_homes_reenters_reads_and_keeps_audit_bytes_distinct() {
    let (database, current) = fixture();
    let catalog = database.catalog();
    let home_schema = schema("home");
    let home_graph = graph("home", "preferred");
    catalog
        .create_schema(&home_schema, CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&home_graph, None, CreatePolicy::Strict)
        .unwrap();

    let configured = principal()
        .with_audit_bytes(Vec::from(&b"opaque-audit"[..]))
        .with_home_schema(home_schema.clone())
        .with_home_graph(home_graph.clone());
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(TestProvider {
        response: ProviderResponse::Principal(Box::new(configured)),
        calls: Arc::clone(&provider_calls),
        reenter: Some((catalog.clone(), home_schema.clone())),
    });
    let policy_calls = Arc::new(AtomicUsize::new(0));
    let policy = Arc::new(TestPolicy {
        response: PolicyResponse::Allow,
        calls: Arc::clone(&policy_calls),
        reenter: Some((catalog, home_schema.clone())),
        expect_homes: true,
    });
    let options = SessionOptions::new()
        .with_authorization_id(AuthorizationId::new("secret-auth").unwrap())
        .with_principal_provider(provider)
        .with_authorization_policy(policy);

    let session = database.session_with_options(&current, options).unwrap();
    assert_eq!(provider_calls.load(Ordering::Relaxed), 1);
    assert_eq!(policy_calls.load(Ordering::Relaxed), 1);
    let context = session.context();
    assert_eq!(context.authorization_id().unwrap().as_str(), "secret-auth");
    let resolved = context.principal().unwrap();
    assert_eq!(resolved.id().as_str(), "secret-principal");
    assert_eq!(resolved.audit_bytes(), Some(&b"opaque-audit"[..]));
    assert_ne!(
        resolved.id().as_str().as_bytes(),
        resolved.audit_bytes().unwrap()
    );
    assert_eq!(context.home_schema().unwrap().path, home_schema);
    assert_eq!(context.home_graph().unwrap().path, home_graph);
    assert_eq!(
        context.dependencies().home_schema(),
        context.home_schema().map(|descriptor| descriptor.id)
    );
    assert_eq!(
        context.dependencies().home_graph(),
        context.home_graph().map(|descriptor| descriptor.id)
    );
    session
        .execute("INSERT (:Authenticated)")
        .expect("audit principal path executes a write");
}

#[test]
fn home_schema_without_a_home_graph_is_valid() {
    let (database, current) = fixture();
    let home = schema("home_only");
    database
        .catalog()
        .create_schema(&home, CreatePolicy::Strict)
        .unwrap();
    let session = database
        .session_with_options(
            &current,
            options_with_principal(principal().with_home_schema(home.clone())),
        )
        .unwrap();

    assert_eq!(session.context().home_schema().unwrap().path, home);
    assert!(session.context().home_graph().is_none());
}

#[test]
fn provider_missing_and_failure_are_structured_and_redacted() {
    let (database, current) = fixture();
    for (response, expected, has_source) in [
        (
            ProviderResponse::Missing,
            ErrorKind::PrincipalNotFound,
            false,
        ),
        (
            ProviderResponse::Failure,
            ErrorKind::PrincipalProviderFailure,
            true,
        ),
    ] {
        let (provider, calls) = TestProvider::with_response(response);
        let options = SessionOptions::new()
            .with_authorization_id(AuthorizationId::new("secret-auth").unwrap())
            .with_principal_provider(provider);
        let error = database
            .session_with_options(&current, options)
            .err()
            .expect("provider path fails");
        assert_eq!(error.kind(), expected);
        assert_eq!(error.source().is_some(), has_source);
        assert!(!error.message().contains("secret"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn policy_denial_and_failure_are_structured_and_redacted() {
    let (database, current) = fixture();
    for (response, expected, has_source) in [
        (PolicyResponse::Deny, ErrorKind::AuthorizationDenied, false),
        (
            PolicyResponse::Failure,
            ErrorKind::AuthorizationPolicyFailure,
            true,
        ),
    ] {
        let (provider, _) = TestProvider::principal(principal());
        let (policy, calls) = TestPolicy::new(response);
        let options = SessionOptions::new()
            .with_authorization_id(AuthorizationId::new("secret-auth").unwrap())
            .with_principal_provider(provider)
            .with_authorization_policy(policy);
        let error = database
            .session_with_options(&current, options)
            .err()
            .expect("policy path fails");
        assert_eq!(error.kind(), expected);
        assert_eq!(error.source().is_some(), has_source);
        assert!(!error.message().contains("secret"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn authorization_and_principal_ids_reject_empty_text() {
    let Err(authorization) = AuthorizationId::new("") else {
        panic!("empty authorization ID must fail");
    };
    assert_eq!(authorization.kind(), ErrorKind::InvalidAuthorizationId);
    let Err(principal) = PrincipalId::new("") else {
        panic!("empty principal ID must fail");
    };
    assert_eq!(principal.kind(), ErrorKind::InvalidPrincipalId);
}

#[test]
fn invalid_missing_and_wrong_kind_homes_are_rejected() {
    let (database, current) = fixture();
    let catalog = database.catalog();
    for name in ["home", "other"] {
        catalog
            .create_schema(&schema(name), CreatePolicy::Strict)
            .unwrap();
    }
    let other_graph = graph("other", "g");
    catalog
        .create_graph(&other_graph, None, CreatePolicy::Strict)
        .unwrap();
    let type_path = graph("home", "shape");
    let label = PathSegment::regular("Person").unwrap();
    let definition = GraphTypeDefinition::builder()
        .with_node_type(NodeTypeDefinition::new(label.clone(), vec![label]).unwrap())
        .build()
        .unwrap();
    catalog
        .create_graph_type(&type_path, definition, CreatePolicy::Strict)
        .unwrap();

    let invalid = [
        principal().with_home_graph(other_graph.clone()),
        principal()
            .with_home_schema(schema("home"))
            .with_home_graph(other_graph),
        principal().with_home_schema(schema("missing")),
        principal()
            .with_home_schema(schema("home"))
            .with_home_graph(graph("home", "missing")),
        principal()
            .with_home_schema(schema("home"))
            .with_home_graph(type_path),
    ];
    for principal in invalid {
        let error = database
            .session_with_options(&current, options_with_principal(principal))
            .err()
            .expect("invalid home fails");
        assert_eq!(error.kind(), ErrorKind::InvalidPrincipalHome);
        assert!(!error.message().contains("secret"));
    }
}

#[test]
fn current_and_home_graph_replacements_invalidate_by_stable_id() {
    let (database, current) = fixture();
    let catalog = database.catalog();
    let current_session = database.session(&current).unwrap();
    catalog
        .create_graph(&current, None, CreatePolicy::OrReplace)
        .unwrap();
    assert_eq!(
        current_session.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );

    let home_schema = schema("home_graph_stale");
    let home_graph = graph("home_graph_stale", "g");
    catalog
        .create_schema(&home_schema, CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&home_graph, None, CreatePolicy::Strict)
        .unwrap();
    let session = database
        .session_with_options(
            &current,
            options_with_principal(
                principal()
                    .with_home_schema(home_schema)
                    .with_home_graph(home_graph.clone()),
            ),
        )
        .unwrap();
    catalog
        .create_graph(&home_graph, None, CreatePolicy::OrReplace)
        .unwrap();
    assert_eq!(
        session.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
}

#[test]
fn dropped_and_recreated_home_schema_does_not_rebind() {
    let (database, current) = fixture();
    let catalog = database.catalog();
    let home = schema("home_schema_stale");
    catalog.create_schema(&home, CreatePolicy::Strict).unwrap();
    let session = database
        .session_with_options(
            &current,
            options_with_principal(principal().with_home_schema(home.clone())),
        )
        .unwrap();

    catalog.drop_schema(&home, DropPolicy::Strict).unwrap();
    catalog.create_schema(&home, CreatePolicy::Strict).unwrap();
    assert_eq!(
        session.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleSessionReference
    );
}

#[test]
fn unrelated_catalog_publication_does_not_mutate_or_invalidate_context() {
    let (database, current) = fixture();
    let session = database.session(&current).unwrap();
    let observed_generation = session.context().catalog_generation();
    database
        .catalog()
        .create_schema(&schema("unrelated"), CreatePolicy::Strict)
        .unwrap();

    assert_eq!(session.context().catalog_generation(), observed_generation);
    session.execute("RETURN 1").unwrap();
}
