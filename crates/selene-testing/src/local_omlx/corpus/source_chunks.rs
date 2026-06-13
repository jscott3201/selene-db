//! Source chunk corpus built from target-aware selene-db implementation snippets.

use super::{CorpusInput, Topic};

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    push_docs(&mut inputs, Topic::Gql, gql_docs());
    push_docs(&mut inputs, Topic::Vector, vector_docs());
    push_docs(&mut inputs, Topic::AgentMemory, memory_docs());
    push_docs(&mut inputs, Topic::Code, code_docs());
    inputs.extend(queries());
    inputs
}

fn push_docs(
    inputs: &mut Vec<CorpusInput>,
    topic: Topic,
    docs: &[(&'static str, Option<&'static str>)],
) {
    inputs.extend(
        docs.iter()
            .map(|(text, target_key)| CorpusInput::document(topic, *text, *target_key)),
    );
}

fn gql_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "GQL chunk roots cover runtime sessions, native built-in dispatch, text scoring, \
             and batched query-root materialization.",
            None,
        ),
        (
            "A second GQL graph-root hint mentions parser lowering, analyzer checks, and CALL \
             execution without naming any target implementation chunk.",
            None,
        ),
        (
            r#"crates/selene-gql/src/runtime/session.rs
pub fn with_plan_cache(mut self, capacity: NonZeroUsize) -> Self {
    self.plan_cache = Some(PlanCache::new(capacity));
    self
}"#,
            Some("chunk-gql-session-plan-cache"),
        ),
        (
            r#"crates/selene-gql/src/runtime/builtins/catalog.rs
pub(super) enum BuiltinKind {
    VectorScoreNodes,
    VectorScoreNodesBatch,
    TextScoreNodes,
    TextScoreNodesBatch,
    TextScoreCandidateStateExpandedBatch,
}"#,
            Some("chunk-gql-builtin-kind-text-vector"),
        ),
        (
            r#"crates/selene-gql/src/runtime/builtins/text_search.rs
const SCORE_PROC_NAME: &str = "selene.text_score_nodes";
const SCORE_BATCH_PROC_NAME: &str = "selene.text_score_nodes_batch";
const SCORE_STATE_EXPANDED_BATCH_PROC_NAME: &str =
    "selene.text_score_candidate_state_expanded_batch";"#,
            Some("chunk-gql-text-score-proc-names"),
        ),
        (
            r#"crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/fixture/text_exec.rs
MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc)
WITH anchor, collect_list(root) AS root_set
CALL selene.text_score_candidate_state_expanded_batch(...)"#,
            Some("chunk-gql-query-root-text-state"),
        ),
        (
            "A nearby registry note talks about procedure metadata and output columns, but it \
             does not define the source-string plan cache constructor.",
            None,
        ),
        (
            "The parser and analyzer lower MATCH clauses before runtime dispatch, but they do \
             not own text_score_candidate_state_expanded_batch execution.",
            None,
        ),
    ]
}

fn vector_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Vector chunk roots cover first-class Value::Vector storage, metric binding, \
             candidate canonicalization, and exact expanded reranking.",
            None,
        ),
        (
            "A second vector graph-root hint covers ANN registration, exact rerank, and graph \
             candidate scoring while staying separate from any target snippet.",
            None,
        ),
        (
            r#"crates/selene-core/src/value.rs
pub enum Value {
    Uuid(uuid::Uuid),
    Vector(VectorValue),
}
pub const MAX_VECTOR_DIMENSION: usize = u16::MAX as usize;"#,
            Some("chunk-vector-value-variant"),
        ),
        (
            r#"crates/selene-core/src/vector.rs
pub fn distance(&self, candidate: &VectorValue) -> CoreResult<f64> {
    check_same_dimension(query.len(), candidate.len())?;
    Ok(canonical_score(match self.metric {
        VectorMetric::SquaredEuclidean => squared_euclidean(query, candidate),
        VectorMetric::Cosine => cosine_distance_with_lhs_norm(...)?,
        VectorMetric::NegativeInnerProduct => -dot(query, candidate),
    }))
}"#,
            Some("chunk-vector-bound-distance"),
        ),
        (
            r#"crates/selene-graph/src/vector_search/types.rs
pub fn from_nodes(nodes: impl IntoIterator<Item = NodeId>) -> Self {
    let mut nodes = nodes.into_iter().collect::<Vec<_>>();
    nodes.sort_unstable();
    nodes.dedup();
    Self { nodes }
}"#,
            Some("chunk-vector-candidate-set-canonical"),
        ),
        (
            r#"crates/selene-graph/src/vector_search/score.rs
pub fn score_vector_expanded_candidate_sets_batch_checked(
    &self,
    property: &DbString,
    queries: &[VectorValue],
    root_sets: &[VectorCandidateSet],
    options: VectorNeighborSearchOptions<'_>,
) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError>"#,
            Some("chunk-vector-expanded-batch-scorer"),
        ),
        (
            "The vector index builder derives Flat, HNSW, and IVF accelerators from graph \
             values, but it is not the canonical NodeId sorting wrapper.",
            None,
        ),
        (
            "A BM25 text index has postings and document lengths; it shares retrieval goals \
             with vector scoring but not the VectorValue metric kernel.",
            None,
        ),
    ]
}

fn memory_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Agent-memory chunk roots cover maintained candidate-state specs, required and \
             excluded edge labels, recovery rebuilds, and provider-owned candidate sets.",
            None,
        ),
        (
            "A second memory graph-root hint describes currentness, provenance, and negative \
             evidence as policy-neutral graph state rather than a target chunk.",
            None,
        ),
        (
            r#"crates/selene-graph/src/candidate_state.rs
pub struct CandidateStateSpec {
    pub name: DbString,
    pub required_label: Option<DbString>,
    pub require_outgoing: Vec<DbString>,
    pub require_incoming: Vec<DbString>,
    pub exclude_outgoing: Vec<DbString>,
    pub exclude_incoming: Vec<DbString>,
}"#,
            Some("chunk-memory-state-spec-fields"),
        ),
        (
            r#"crates/selene-graph/src/candidate_state.rs
pub fn require_outgoing(mut self, label: DbString) -> Self {
    insert_sorted_unique(&mut self.require_outgoing, label);
    self
}
pub fn exclude_outgoing(mut self, label: DbString) -> Self {
    insert_sorted_unique(&mut self.exclude_outgoing, label);
    self
}"#,
            Some("chunk-memory-state-edge-rules"),
        ),
        (
            r#"crates/selene-graph/src/candidate_state.rs
fn apply_change(&mut self, specs: &[CandidateStateSpec], change: &Change) -> Result<(), ProviderError> {
    match change {
        Change::NodeDeleted { id } => self.remove_node(specs, *id),
        Change::EdgeDeleted { id } => self.remove_edge(specs, *id),
        _ => self.rebuild_derived(specs),
    }
}"#,
            Some("chunk-memory-state-apply-change"),
        ),
        (
            r#"crates/selene-graph/src/candidate_state_shared.rs
pub fn vector_candidate_set(
    &self,
    name: &DbString,
    snapshot: &SeleneGraph,
) -> Result<Option<VectorCandidateSet>, ProviderError> {
    provider.vector_candidate_set(name, snapshot.meta.generation)
}"#,
            Some("chunk-memory-shared-generation-check"),
        ),
        (
            "Currentness and provenance labels can narrow memory retrieval before text or \
             vector scoring, but they do not choose one ranking policy for the engine.",
            None,
        ),
        (
            "A stale support fact can remain in the graph for audit and provenance while a \
             maintained current-state set excludes it from retrieval.",
            None,
        ),
    ]
}

fn code_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Workflow chunk roots cover local embedding providers, benchmark runner hygiene, \
             WAL append policy, and graph mixed workload rows.",
            None,
        ),
        (
            "A second workflow graph-root hint mentions CI-safe local benchmarks and ignored \
             API keys without identifying any concrete target snippet.",
            None,
        ),
        (
            r#"crates/selene-testing/src/local_omlx/config.rs
pub enum EmbeddingProvider {
    Omlx,
    OpenRouter,
}
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
    pub corpus: CorpusProfile,
    pub batch_size: usize,
}"#,
            Some("chunk-code-embedding-config"),
        ),
        (
            r#"crates/selene-testing/src/local_omlx/client.rs
pub struct OpenRouterEmbeddingClient {
    api_key: String,
    model: String,
}
impl EmbeddingClient for OpenRouterEmbeddingClient {
    fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> { ... }
}"#,
            Some("chunk-code-openrouter-client"),
        ),
        (
            r#"scripts/run-benches.sh
run_cargo_bench() {
  local bench_name="$1"
  local filter="$2"
  cargo bench -p "$crate" --bench "$bench_name" -- "$filter"
}"#,
            Some("chunk-code-bench-runner"),
        ),
        (
            r#"crates/selene-persist/src/writer.rs
pub fn append(
    &mut self,
    timestamp: HlcTimestamp,
    origin: Origin,
    principal: Option<Arc<[u8]>>,
    changes: &[Change],
) -> Result<u64, PersistError>"#,
            Some("chunk-code-wal-append"),
        ),
        (
            "A benchmark documentation check verifies registered Criterion targets, but it does \
             not read embedding provider API keys.",
            None,
        ),
        (
            "The graph mixed workload row interleaves snapshot point reads and property-update \
             commits, but it deliberately excludes WAL and vector-index maintenance.",
            None,
        ),
    ]
}

fn queries() -> [CorpusInput; 16] {
    [
        query(
            Topic::Gql,
            "Which runtime chunk enables source-string plan cache reuse for Session execution?",
            "chunk-gql-session-plan-cache",
        ),
        query(
            Topic::Gql,
            "Which built-in catalog chunk lists both vector scoring and BM25 text scoring procedure kinds?",
            "chunk-gql-builtin-kind-text-vector",
        ),
        query(
            Topic::Gql,
            "Which native built-in chunk names selene.text_score_nodes_batch and the state-expanded BM25 batch procedure?",
            "chunk-gql-text-score-proc-names",
        ),
        query(
            Topic::Gql,
            "Which query-root benchmark chunk collects roots and calls the state-expanded text scorer?",
            "chunk-gql-query-root-text-state",
        ),
        query(
            Topic::Vector,
            "Which core value chunk makes dense embeddings a real Value::Vector variant?",
            "chunk-vector-value-variant",
        ),
        query(
            Topic::Vector,
            "Which vector metric chunk binds a query and computes lower-is-better cosine or inner-product distance?",
            "chunk-vector-bound-distance",
        ),
        query(
            Topic::Vector,
            "Which vector search chunk sorts and deduplicates NodeId candidates into a canonical set?",
            "chunk-vector-candidate-set-canonical",
        ),
        query(
            Topic::Vector,
            "Which graph vector chunk batches graph-expanded root candidate scoring for many query vectors?",
            "chunk-vector-expanded-batch-scorer",
        ),
        query(
            Topic::AgentMemory,
            "Which candidate-state chunk defines required labels plus required and excluded edge-label vectors?",
            "chunk-memory-state-spec-fields",
        ),
        query(
            Topic::AgentMemory,
            "Which maintained-state chunk inserts required and excluded outgoing edge rules in sorted unique order?",
            "chunk-memory-state-edge-rules",
        ),
        query(
            Topic::AgentMemory,
            "Which maintained-state chunk applies node and edge delete changes before rebuilding derived members?",
            "chunk-memory-state-apply-change",
        ),
        query(
            Topic::AgentMemory,
            "Which shared candidate-state chunk checks provider generation before returning a VectorCandidateSet?",
            "chunk-memory-shared-generation-check",
        ),
        query(
            Topic::Code,
            "Which local embedding chunk reads provider, corpus, model, and batch-size configuration?",
            "chunk-code-embedding-config",
        ),
        query(
            Topic::Code,
            "Which local embedding client chunk owns the OpenRouter API key and model for Codestral embeddings?",
            "chunk-code-openrouter-client",
        ),
        query(
            Topic::Code,
            "Which benchmark runner chunk serializes cargo bench invocations through a named bench target?",
            "chunk-code-bench-runner",
        ),
        query(
            Topic::Code,
            "Which persistence chunk appends Change batches to the WAL with timestamp, origin, and principal?",
            "chunk-code-wal-append",
        ),
    ]
}

fn query(topic: Topic, text: &'static str, target_key: &'static str) -> CorpusInput {
    CorpusInput::query(topic, text, Some(target_key))
}
