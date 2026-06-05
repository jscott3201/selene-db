//! Target-aware corpus built from short selene-db source excerpts.

use super::{CorpusInput, Topic};

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    push_docs(
        &mut inputs,
        Topic::Gql,
        &[
            (
                "runtime/query roots: Session, ProcedureRegistry, BuiltinKind, and text scoring \
                 snippets all describe native ISO GQL CALL execution.",
                None,
            ),
            (
                "GQL benchmark roots collect OmlxQueryAnchor rows, group root sets, and call \
                 selene procedures without adding grammar shortcuts.",
                None,
            ),
            (
                r#"crates/selene-gql/src/runtime/session.rs
pub fn with_plan_cache(mut self, capacity: NonZeroUsize) -> Self {
    self.plan_cache = Some(PlanCache::new(capacity));
    self
}"#,
                Some("src-gql-session-plan-cache"),
            ),
            (
                r#"crates/selene-gql/src/runtime/call_plan_cache.rs
pub struct CallPlanCache {
    inner: Mutex<CallPlanCacheInner>,
}
struct CallPlanCacheInner {
    plans: LruCache<CallPlanKey, Arc<ExecutionPlan>>,
    source_index: LruCache<Arc<str>, Vec<CallPlanSourceEntry>>,
    stats: CallPlanCacheStats,
}"#,
                Some("src-gql-call-plan-cache"),
            ),
            (
                r#"crates/selene-gql/src/runtime/builtins/catalog.rs
enum BuiltinKind {
    TextScoreNodesBatch,
    TextScoreCandidateStateExpandedBatch,
    VectorScoreCandidateStateExpandedBatch,
}"#,
                Some("src-gql-builtin-text-state"),
            ),
            (
                r#"crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/fixture/text_exec.rs
const QUERY_ROOT_CURRENT_STATE_TEXT_SCORE_BATCH_SOURCE: &str =
    "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc)
     WITH collect_list(root) AS root_sets
     CALL selene.text_score_candidate_state_expanded_batch(...)";"#,
                Some("src-gql-query-root-text-state"),
            ),
            (
                "A parser-cache note mentions source reuse and native calls, but it is a stale \
                 planning note and is excluded from current support.",
                None,
            ),
            (
                "Procedure docs describe argument names and yield columns for text scoring, but \
                 not the runtime cache object itself.",
                None,
            ),
            (
                "Analyzer and optimizer modules lower MATCH and RETURN clauses before runtime \
                 dispatch sees procedure calls.",
                None,
            ),
            (
                "A superseded registry sketch used external extension packs; current source keeps \
                 built-ins in the native registry.",
                None,
            ),
        ],
    );
    push_docs(
        &mut inputs,
        Topic::Vector,
        &[
            (
                "vector/source roots: value storage, metric scoring, candidate canonicalization, \
                 and graph-expanded batch scoring form the retrieval path.",
                None,
            ),
            (
                "Embedding procedures turn graph roots into candidate sets, then exact rerank \
                 scores first-class VectorValue properties.",
                None,
            ),
            (
                r#"crates/selene-core/src/value.rs
pub enum Value {
    Uuid(uuid::Uuid),
    Vector(VectorValue),
}
pub const MAX_VECTOR_DIMENSION: usize = u16::MAX as usize;"#,
                Some("src-vector-value-variant"),
            ),
            (
                r#"crates/selene-core/src/vector.rs
pub fn distance(&self, candidate: &VectorValue) -> CoreResult<f64> {
    let query = self.query.as_slice();
    let candidate = candidate.as_slice();
    check_same_dimension(query.len(), candidate.len())?;
    Ok(canonical_score(match self.metric {
        VectorMetric::SquaredEuclidean => squared_euclidean(query, candidate),
        VectorMetric::Cosine => cosine_distance_with_lhs_norm(
            query,
            candidate,
            self.query_norm.expect("cosine query scorer stores query norm"),
        )?,
        VectorMetric::NegativeInnerProduct => -dot(query, candidate),
    }))
}"#,
                Some("src-vector-bound-distance"),
            ),
            (
                r#"crates/selene-graph/src/vector_search/types.rs
pub struct VectorCandidateSet {
    nodes: Vec<NodeId>,
}
pub fn from_nodes(nodes: impl IntoIterator<Item = NodeId>) -> Self {
    let mut nodes = nodes.into_iter().collect::<Vec<_>>();
    nodes.sort_unstable();
    nodes.dedup();
    Self { nodes }
}"#,
                Some("src-vector-candidate-set"),
            ),
            (
                r#"crates/selene-graph/src/vector_search/score.rs
pub fn score_vector_expanded_candidate_sets_batch_checked(
    &self,
    property: &IStr,
    queries: &[VectorValue],
    root_sets: &[VectorCandidateSet],
    options: VectorNeighborSearchOptions<'_>,
    checker: CancellationChecker<'_>,
) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError>"#,
                Some("src-vector-expanded-batch"),
            ),
            (
                "A contradicted vector benchmark note suggests adding ANN candidates after precise \
                 graph expansion; current rows keep that as negative evidence.",
                None,
            ),
            (
                "HNSW and IVF index registration snippets discuss approximate roots, not the \
                 exact candidate-set batch scorer.",
                None,
            ),
            (
                "Vector docs mention cosine, dot product, and Euclidean distance in prose without \
                 showing the bound query scorer.",
                None,
            ),
            (
                "A stale candidate cleaning helper rebuilt caller lists directly before the \
                 VectorCandidateSet wrapper owned canonicalization.",
                None,
            ),
        ],
    );
    push_docs(
        &mut inputs,
        Topic::AgentMemory,
        &[
            (
                "agent-memory/source roots: maintained candidate state, negative edges, required \
                 provenance, and provider-owned sets shape current recall.",
                None,
            ),
            (
                "Memory retrieval samples combine graph-derived support facts with text and vector \
                 scorers through policy-neutral procedures.",
                None,
            ),
            (
                r#"crates/selene-graph/src/candidate_state.rs
pub struct CandidateStateSpec {
    pub name: IStr,
    pub required_label: Option<IStr>,
    pub require_outgoing: Vec<IStr>,
    pub require_incoming: Vec<IStr>,
    pub exclude_outgoing: Vec<IStr>,
    pub exclude_incoming: Vec<IStr>,
}"#,
                Some("src-memory-state-spec"),
            ),
            (
                r#"crates/selene-graph/src/candidate_state.rs
pub fn exclude_outgoing(mut self, edge_label: IStr) -> Self {
    self.exclude_outgoing.push(edge_label);
    self
}"#,
                Some("src-memory-exclude-outgoing"),
            ),
            (
                r#"crates/selene-graph/src/candidate_state.rs
pub fn require_incoming(mut self, edge_label: IStr) -> Self {
    self.require_incoming.push(edge_label);
    self
}
pub fn require_outgoing(mut self, edge_label: IStr) -> Self {
    self.require_outgoing.push(edge_label);
    self
}"#,
                Some("src-memory-required-edges"),
            ),
            (
                r#"crates/selene-graph/src/candidate_state.rs
pub fn candidate_set(&self, name: &IStr) -> Option<VectorCandidateSet> {
    self.state.lock().members.get(name).map(|members| {
        VectorCandidateSet::from_canonical_nodes(members.iter().copied().collect())
    })
}"#,
                Some("src-memory-provider-candidate-set"),
            ),
            (
                "A stale memory note has useful provenance words, but negative evidence should \
                 remove it from omlx_current_support_facts.",
                None,
            ),
            (
                "A superseded support-state sketch stored active facts outside the graph; current \
                 provider state is rebuildable from graph changes.",
                None,
            ),
            (
                "Candidate-state discovery returns names, generations, labels, and edge rules for \
                 callers that compose retrieval policies.",
                None,
            ),
            (
                "Graph expansion can find support facts, but currentness and provenance are separate \
                 maintained-state decisions.",
                None,
            ),
        ],
    );
    push_docs(
        &mut inputs,
        Topic::Code,
        &[
            (
                "code/source roots: embedding config, OpenRouter setup, benchmark serialization, \
                 and documentation guards keep local rows reproducible.",
                None,
            ),
            (
                "Benchmark helper snippets use ignored environment variables and provider-free CI \
                 tests before running optional remote embeddings.",
                None,
            ),
            (
                r#"crates/selene-testing/src/local_omlx/client.rs
pub struct OpenRouterClient {
    endpoint: String,
    api_key: String,
    batch_size: usize,
    referer: String,
    title: String,
}"#,
                Some("src-code-openrouter-client"),
            ),
            (
                r#"crates/selene-testing/src/local_omlx/config.rs
pub struct EmbeddingBenchConfig {
    pub provider: EmbeddingProvider,
    pub models: Vec<String>,
    pub corpus: CorpusProfile,
    pub batch_size: usize,
    pub graph_hint_docs_per_topic: Option<usize>,
    client: EmbeddingClient,
}"#,
                Some("src-code-embedding-config"),
            ),
            (
                r#"scripts/run-benches.sh
if pgrep -f "cargo bench" | grep -v "$$" >/dev/null 2>&1; then
  echo "ERROR: detected another cargo bench process"
  exit 1
fi"#,
                Some("src-code-run-benches-conflict"),
            ),
            (
                r#".github/scripts/check-benchmarks-doc.sh
for bench in $benches; do
  if ! grep -Fq "$bench" "$doc"; then
    echo "FAIL: BENCHMARKS.md is missing registered bench '$bench'"
  fi
done"#,
                Some("src-code-bench-doc-guard"),
            ),
            (
                "A stale local benchmark command printed environment details; current code must \
                 never expose ignored embedding API keys.",
                None,
            ),
            (
                "A superseded HTTP experiment added an async dependency; current OpenRouter setup \
                 shells out to curl before Criterion timing.",
                None,
            ),
            (
                "Cargo clippy and file-size checks cover benchmark code as well as workspace crates.",
                None,
            ),
            (
                "Goal logs keep local benchmark evidence outside git while tracked docs record stable \
                 benchmark commands.",
                None,
            ),
        ],
    );
    inputs.extend(queries());
    inputs
}

fn push_docs(
    inputs: &mut Vec<CorpusInput>,
    topic: Topic,
    docs: &[(&'static str, Option<&'static str>)],
) {
    inputs.extend(docs.iter().map(|(text, target_key)| CorpusInput {
        topic,
        is_document: true,
        text,
        target_key: *target_key,
    }));
}

fn queries() -> [CorpusInput; 16] {
    [
        query(
            Topic::Gql,
            "Which Session method keeps repeated GQL source strings from being parsed and planned again?",
            "src-gql-session-plan-cache",
        ),
        query(
            Topic::Gql,
            "Which runtime cache stores native CALL plans keyed by procedure argument shape?",
            "src-gql-call-plan-cache",
        ),
        query(
            Topic::Gql,
            "Which BuiltinKind entry routes batched BM25 over maintained candidate state and expanded roots?",
            "src-gql-builtin-text-state",
        ),
        query(
            Topic::Gql,
            "Which query-root text source collects roots before calling current-state BM25 batch scoring?",
            "src-gql-query-root-text-state",
        ),
        query(
            Topic::Vector,
            "Where does the core Value enum store first-class dense embedding vectors?",
            "src-vector-value-variant",
        ),
        query(
            Topic::Vector,
            "Which bound vector scorer dispatches cosine, negative inner product, and squared Euclidean distance?",
            "src-vector-bound-distance",
        ),
        query(
            Topic::Vector,
            "Which type sorts and deduplicates NodeId inputs before exact vector reranking?",
            "src-vector-candidate-set",
        ),
        query(
            Topic::Vector,
            "Which graph scoring API expands root candidate sets once for a batch of query vectors?",
            "src-vector-expanded-batch",
        ),
        query(
            Topic::AgentMemory,
            "Which spec stores required labels plus incoming, outgoing, and exclusion edge rules?",
            "src-memory-state-spec",
        ),
        query(
            Topic::AgentMemory,
            "Which candidate-state builder method removes nodes with disqualifying outgoing edges?",
            "src-memory-exclude-outgoing",
        ),
        query(
            Topic::AgentMemory,
            "Which builder methods require provenance edges entering and leaving a candidate memory fact?",
            "src-memory-required-edges",
        ),
        query(
            Topic::AgentMemory,
            "Which provider method returns a named maintained state as a VectorCandidateSet?",
            "src-memory-provider-candidate-set",
        ),
        query(
            Topic::Code,
            "Which local embedding helper owns the OpenRouter endpoint, API key, batch size, referer, and title?",
            "src-code-openrouter-client",
        ),
        query(
            Topic::Code,
            "Which benchmark config struct carries the embedding client, model list, corpus, and graph hint settings?",
            "src-code-embedding-config",
        ),
        query(
            Topic::Code,
            "Which benchmark runner guard rejects concurrent cargo bench processes?",
            "src-code-run-benches-conflict",
        ),
        query(
            Topic::Code,
            "Which repository script fails when a registered benchmark is missing from BENCHMARKS.md?",
            "src-code-bench-doc-guard",
        ),
    ]
}

fn query(topic: Topic, text: &'static str, target_key: &'static str) -> CorpusInput {
    CorpusInput {
        topic,
        is_document: false,
        text,
        target_key: Some(target_key),
    }
}
